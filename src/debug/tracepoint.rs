use super::expr::Expr;
use crate::{Emulator, Event, Result};
use std::{cell::RefCell, collections::BTreeMap, rc::Rc};
use unicorn_engine::UcHookId;

#[cfg(test)]
mod tests;

#[derive(Clone, Debug)]
pub struct Tracepoint {
    pub values: Vec<String>,
    pub log: bool,
}

#[derive(Default)]
pub(crate) struct Tracepoints {
    points: BTreeMap<u64, (Rc<RefCell<Point>>, UcHookId)>,
    stale: bool,
}
struct Point {
    values: Vec<(String, Expr)>,
    log: bool,
    hits: BTreeMap<u64, u64>,
}

impl Tracepoints {
    pub(crate) fn mark_stale(&mut self) {
        self.stale = true;
    }
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
        self.install_tracepoint(
            address & !1,
            Rc::new(RefCell::new(Point {
                values,
                log: tracepoint.log,
                hits: BTreeMap::new(),
            })),
        )
    }

    pub fn remove_tracepoint(&mut self, address: u64) -> Result<()> {
        let address = address & !1;
        let hook = self.tracepoint(address)?.1;
        self.remove_address_hook(address, hook)?;
        self.tracepoints.points.remove(&address);
        Ok(())
    }

    pub fn tracepoint_hits(&self, address: u64) -> Result<Vec<(u64, u64)>> {
        let (point, _) = self.tracepoint(address)?;
        Ok(point
            .borrow()
            .hits
            .iter()
            .map(|(&value, &count)| (value, count))
            .collect())
    }

    pub fn reset_tracepoint_hits(&mut self) {
        for (point, _) in self.tracepoints.points.values() {
            point.borrow_mut().hits.clear();
        }
    }

    pub(crate) fn reset_tracepoint(&mut self, address: u64) -> Result<()> {
        self.tracepoint(address)?.0.borrow_mut().hits.clear();
        Ok(())
    }

    fn tracepoint(&self, address: u64) -> Result<&(Rc<RefCell<Point>>, UcHookId)> {
        let address = address & !1;
        self.tracepoints
            .points
            .get(&address)
            .ok_or_else(|| format!("no tracepoint at {}", self.symbolize(address)).into())
    }

    pub(crate) fn refresh_tracepoints(&mut self) -> Result<()> {
        if !self.tracepoints.stale {
            return Ok(());
        }
        let points = self.tracepoints.points.clone();
        for (address, (point, _)) in points {
            self.install_tracepoint(address, point)?;
        }
        self.tracepoints.stale = false;
        Ok(())
    }

    fn install_tracepoint(&mut self, address: u64, point: Rc<RefCell<Point>>) -> Result<()> {
        let events = self.events.clone();
        let modules = self.module_table.clone();
        let previous = self.tracepoints.points.get(&address).map(|(_, hook)| *hook);
        let callback = point.clone();
        let hook = self.install_tracepoint_hook(address, move |cpu, pc, _| {
            if pc != address {
                return;
            }
            let mut point = callback.borrow_mut();
            let first = point.values[0].1.value(cpu);
            if let Some(value) = first {
                *point.hits.entry(value).or_default() += 1;
            }
            if point.log {
                let at = modules.borrow().symbolize(address);
                let values = point
                    .values
                    .iter()
                    .enumerate()
                    .map(|(i, (text, expr))| {
                        (text.clone(), if i == 0 { first } else { expr.value(cpu) })
                    })
                    .collect();
                events.borrow_mut().emit(Event::Tracepoint {
                    at,
                    address,
                    thread: cpu.get_data().thread,
                    values,
                });
            }
        })?;
        // Installation already invalidated this range; keep replacement atomic.
        let cpu = self.cpu_mut()?;
        if let Some(previous) = previous
            && let Err(error) = cpu.remove_hook(previous)
        {
            cpu.remove_hook(hook)?;
            return Err(error.into());
        }
        self.tracepoints.points.insert(address, (point, hook));
        Ok(())
    }
}
