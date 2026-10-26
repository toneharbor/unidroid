use super::*;

impl Emulator {
    /// Create a built-in map, inserting entries in the supplied order.
    pub fn new_java_map(&mut self, class: &str, entries: &[(Handle, Handle)]) -> Result<Handle> {
        let kind = maps::MapKind::parse(class).ok_or_else(|| format!("expected java/util/HashMap, java/util/LinkedHashMap or java/util/TreeMap, got {class}"))?;
        let mut map = Map::new(kind, 16);
        for &(key, value) in entries {
            let key = reference(self, key)?;
            let value = reference(self, value)?;
            let hash = object::hash(self, &key);
            map.put(self, key, value, hash)
                .ok_or("expected comparable TreeMap keys")?;
        }
        Ok(self.insert_api_object(Object::Map(map)))
    }
    /// Return fresh host handles in Java iteration order. Release the returned handles after use.
    pub fn java_map(&mut self, handle: Handle) -> Result<Vec<(Handle, Handle)>> {
        let entries = match &*self.jni_object(handle)?.borrow() {
            Object::Map(map) => map.ordered(),
            object => {
                return Err(object.handle_error(handle, "a built-in map"));
            }
        };
        Ok(entries
            .into_iter()
            .map(|(key, value)| (self.handle_or_null(key, 0), self.handle_or_null(value, 0)))
            .collect())
    }
    pub fn new_java_list(&mut self, class: &str, items: &[Handle]) -> Result<Handle> {
        if !matches!(class, "java/util/ArrayList" | "java/util/LinkedList") {
            return Err(format!(
                "expected java/util/ArrayList or java/util/LinkedList, got {class}"
            )
            .into());
        }
        let items = items
            .iter()
            .map(|&handle| reference(self, handle))
            .collect::<Result<_>>()?;
        Ok(self.insert_api_object(Object::List(class.into(), items)))
    }
    /// Return fresh host handles in list order. Release the returned handles after use.
    pub fn java_list(&mut self, handle: Handle) -> Result<Vec<Handle>> {
        let items = match &*self.jni_object(handle)?.borrow() {
            Object::List(_, items) => items.clone(),
            object => {
                return Err(object.handle_error(handle, "a built-in list"));
            }
        };
        Ok(items
            .into_iter()
            .map(|item| self.handle_or_null(item, 0))
            .collect())
    }
    pub fn new_boxed(&mut self, value: JniValue) -> Result<Handle> {
        if boxed_class(value).is_none() {
            return Err("expected a primitive value to box, not Object or Void".into());
        }
        Ok(self.insert_api_object(Object::Boxed(value)))
    }
    pub fn unboxed(&self, handle: Handle) -> Result<JniValue> {
        match &*self.jni_object(handle)?.borrow() {
            Object::Boxed(value) => Ok(*value),
            object => Err(object.handle_error(handle, "a boxed primitive")),
        }
    }
}
