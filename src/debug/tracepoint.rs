use super::expr::Expr;
use crate::{Emulator, Event, Result};
use std::{cell::RefCell, collections::BTreeMap, rc::Rc};
use unicorn_engine::UcHookId;

#[derive(Clone, Debug)]
pub struct Tracepoint {
    pub values: Vec<String>,
    pub log: bool,
}

#[derive(Default)]
pub(crate) struct Tracepoints {
    points: BTreeMap<u64, Rc<RefCell<Point>>>,
    hooks: Vec<(u64, UcHookId)>,
}
struct Point {
    values: Vec<(String, Expr)>,
    log: bool,
    hits: BTreeMap<u64, u64>,
}

impl Emulator {
    pub fn add_tracepoint(&mut self, address: u64, tracepoint: &Tracepoint) -> Result<()> {
        if tracepoint.values.is_empty() {
            return Err("tracepoint needs at least one value".into());
        }
        self.cpu()?;
        let values = tracepoint
            .values
            .iter()
            .map(|text| Ok((text.clone(), Expr::parse(self, text)?)))
            .collect::<Result<_>>()?;
        self.tracepoints.points.insert(
            address & !1,
            Rc::new(RefCell::new(Point {
                values,
                log: tracepoint.log,
                hits: BTreeMap::new(),
            })),
        );
        self.refresh_tracepoints()
    }

    pub fn remove_tracepoint(&mut self, address: u64) -> Result<()> {
        self.tracepoints
            .points
            .remove(&(address & !1))
            .ok_or_else(|| format!("no tracepoint at {}", self.symbolize(address & !1)))?;
        self.refresh_tracepoints()
    }

    pub fn tracepoint_hits(&self, address: u64) -> Result<Vec<(u64, u64)>> {
        let point = self
            .tracepoints
            .points
            .get(&(address & !1))
            .ok_or_else(|| format!("no tracepoint at {}", self.symbolize(address & !1)))?;
        Ok(point
            .borrow()
            .hits
            .iter()
            .map(|(&value, &count)| (value, count))
            .collect())
    }

    pub fn reset_tracepoint_hits(&mut self) {
        for point in self.tracepoints.points.values() {
            point.borrow_mut().hits.clear();
        }
    }

    pub(crate) fn reset_tracepoint(&mut self, address: u64) -> Result<()> {
        let point = self
            .tracepoints
            .points
            .get(&(address & !1))
            .ok_or_else(|| format!("no tracepoint at {}", self.symbolize(address & !1)))?;
        point.borrow_mut().hits.clear();
        Ok(())
    }

    pub(crate) fn refresh_tracepoints(&mut self) -> Result<()> {
        for (address, hook) in std::mem::take(&mut self.tracepoints.hooks) {
            let cpu = self.cpu_mut()?;
            cpu.remove_hook(hook)?;
            cpu.ctl_remove_cache(address, address + 4)?;
        }
        let points = self.tracepoints.points.clone();
        for (address, point) in points {
            let at = self.symbolize(address);
            let events = self.events.clone();
            let cpu = self.cpu_mut()?;
            let hook = cpu.add_code_hook(address, address, move |cpu, _, _| {
                let mut point = point.borrow_mut();
                let first = point.values[0].1.value(cpu);
                if let Some(value) = first {
                    *point.hits.entry(value).or_default() += 1;
                }
                if point.log {
                    let values = point
                        .values
                        .iter()
                        .enumerate()
                        .map(|(i, (text, expr))| {
                            (text.clone(), if i == 0 { first } else { expr.value(cpu) })
                        })
                        .collect();
                    events.borrow_mut().emit(Event::Tracepoint {
                        at: at.clone(),
                        address,
                        thread: cpu.get_data().thread,
                        values,
                    });
                }
            })?;
            cpu.ctl_remove_cache(address, address + 4)?;
            self.tracepoints.hooks.push((address, hook));
        }
        Ok(())
    }
}
