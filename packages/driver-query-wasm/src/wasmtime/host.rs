use driver_util::Object;
use wasmtime::component::{Accessor, HasSelf, Resource, bindgen};

bindgen!({
    world: "guest",
});
use polywolf::driver::boundary::*;

use super::value::{InlineValue, OutlineValue};
use crate::QueryContext;

struct State {
    ctx: QueryContext,
    blobs: Vec<Object>,
    values: Vec<OutlineValue>,
}

impl State {
    fn mk_blob(&mut self, blob: Object) -> Resource<Blob> {
        self.blobs.push(blob);
        let idx = self.blobs.len() - 1;
        Resource::new_own(idx.try_into().expect("too many blobs"))
    }

    fn blob(&self, this: Resource<Blob>) -> &Object {
        let idx = this.rep();
        &self.blobs[idx as usize]
    }

    fn mk_value(&mut self, value: OutlineValue) -> Resource<Value> {
        self.values.push(value);
        let idx = self.values.len() - 1;
        Resource::new_own(idx.try_into().expect("too many values"))
    }

    fn value(&self, this: Resource<Value>) -> &OutlineValue {
        let idx = this.rep();
        &self.values[idx as usize]
    }

    fn to_inline_value(&self, value: &OutlineValue) -> InlineValue {
        match value {
            OutlineValue::Null => InlineValue::Null,
            OutlineValue::Bool(b) => InlineValue::Bool(*b),
            OutlineValue::Int(i) => InlineValue::Int(*i),
            OutlineValue::String(s) => InlineValue::String(s.clone()),
            OutlineValue::Array(arr) => InlineValue::Array(
                arr.iter()
                    .map(|value| self.to_inline_value(self.value(value.clone().into())))
                    .collect(),
            ),
            OutlineValue::Object(obj) => InlineValue::Object(
                obj.iter()
                    .map(|(key, value)| {
                        (
                            key.clone(),
                            self.to_inline_value(self.value(value.clone().into())),
                        )
                    })
                    .collect(),
            ),
            OutlineValue::Blob(blob) => InlineValue::Blob(blob.clone()),
        }
    }
}

impl HostBlob for State {
    fn store(&mut self, bytes: Vec<u8>) -> Resource<Blob> {
        let object = self.ctx.store(bytes).expect("storing blob");
        self.mk_blob(object)
    }

    fn store_string(&mut self, s: String) -> Resource<Blob> {
        self.store(s.into_bytes())
    }

    fn load(&mut self, this: Resource<Blob>) -> Vec<u8> {
        let object = self.blob(this);
        self.ctx.load_bytes(object).expect("loading blob")
    }

    fn load_string(&mut self, this: Resource<Blob>) -> String {
        let object = self.blob(this);
        self.ctx
            .load_string(object)
            .expect("loading blob as string")
    }

    fn hash(&mut self, this: Resource<Blob>) -> String {
        let idx = this.rep();
        let object = &self.blobs[idx as usize];
        object.to_string()
    }

    fn drop(&mut self, _this: Resource<Blob>) -> wasmtime::Result<()> {
        // TODO-someday: compaction if the arena is ever really too large. I expect it won't be.
        Ok(())
    }
}

/// This is all just glue code. I might want to stuff it elsewhere just b/c it's so boring.
impl HostValue for State {
    fn kind(&mut self, this: Resource<Value>) -> ValueKind {
        let value = self.value(this);
        match value {
            OutlineValue::Null => ValueKind::Null,
            OutlineValue::Bool(_) => ValueKind::Boolean,
            OutlineValue::Int(_) => ValueKind::Int,
            OutlineValue::String(_) => ValueKind::Str,
            OutlineValue::Array(_) => ValueKind::Array,
            OutlineValue::Object(_) => ValueKind::Object,
            OutlineValue::Blob(_) => ValueKind::Blob,
        }
    }

    fn as_boolean(&mut self, this: Resource<Value>) -> Option<bool> {
        let value = self.value(this);
        match value {
            OutlineValue::Bool(b) => Some(*b),
            _ => None,
        }
    }

    fn as_int(&mut self, this: Resource<Value>) -> Option<i32> {
        let value = self.value(this);
        match value {
            OutlineValue::Int(i) => Some(*i),
            _ => None,
        }
    }

    fn as_str(&mut self, this: Resource<Value>) -> Option<String> {
        let value = self.value(this);
        match value {
            OutlineValue::String(s) => Some(s.clone()),
            _ => None,
        }
    }

    fn as_array(&mut self, this: Resource<Value>) -> Option<Vec<Resource<Value>>> {
        let value = self.value(this);
        match value {
            OutlineValue::Array(arr) => {
                Some(arr.iter().map(|value| value.clone().into()).collect())
            }
            _ => None,
        }
    }

    fn as_object(&mut self, this: Resource<Value>) -> Option<Vec<(String, Resource<Value>)>> {
        let value = self.value(this);
        match value {
            OutlineValue::Object(obj) => Some(
                obj.iter()
                    .map(|(key, value)| (key.clone(), value.clone().into()))
                    .collect(),
            ),
            _ => None,
        }
    }

    fn as_blob(&mut self, this: Resource<Value>) -> Option<Resource<Blob>> {
        let value = self.value(this);
        match value {
            OutlineValue::Blob(blob) => Some(self.mk_blob(blob.clone())),
            _ => None,
        }
    }

    fn drop(&mut self, _this: Resource<Value>) -> wasmtime::Result<()> {
        // TODO-someday: compaction
        Ok(())
    }

    fn of_null(&mut self) -> Resource<Value> {
        self.mk_value(OutlineValue::Null)
    }

    fn of_boolean(&mut self, b: bool) -> Resource<Value> {
        self.mk_value(OutlineValue::Bool(b))
    }

    fn of_int(&mut self, i: i32) -> Resource<Value> {
        self.mk_value(OutlineValue::Int(i))
    }

    fn of_str(&mut self, s: String) -> Resource<Value> {
        self.mk_value(OutlineValue::String(s))
    }

    fn of_array(&mut self, arr: Vec<Resource<Value>>) -> Resource<Value> {
        self.mk_value(OutlineValue::Array(
            arr.into_iter().map(|value| value.into()).collect(),
        ))
    }

    fn of_object(&mut self, obj: Vec<(String, Resource<Value>)>) -> Resource<Value> {
        self.mk_value(OutlineValue::Object(
            obj.into_iter()
                .map(|(key, value)| (key, value.into()))
                .collect(),
        ))
    }

    fn of_blob(&mut self, b: Resource<Blob>) -> Resource<Value> {
        self.mk_value(OutlineValue::Blob(self.blob(b).clone()))
    }
}

impl Host for State {
    fn print(&mut self, text: String) {
        println!("{}", text);
    }

    fn slugify(&mut self, s: String) -> String {
        slug::slugify(s)
    }

    fn write_output(&mut self, path: Path, contents: Resource<Blob>) -> Option<Error> {
        todo!()
    }
}

impl<T> HostWithStore<T> for HasSelf<State> {
    async fn read_file(accessor: &Accessor<T, Self>, path: Path) -> Result<Resource<Blob>, Error> {
        todo!()
    }

    async fn list_directory(accessor: &Accessor<T, Self>, path: Path) -> Result<Vec<Path>, Error> {
        todo!()
    }

    async fn file_type(accessor: &Accessor<T, Self>, path: Path) -> Result<String, Error> {
        todo!()
    }

    async fn get_url(accessor: &Accessor<T, Self>, url: String) -> Result<Resource<Blob>, Error> {
        todo!()
    }

    async fn run(
        accessor: &Accessor<T, Self>,
        wasm: Path,
        num: u8,
        arg: Resource<Value>,
    ) -> Result<Resource<Value>, Error> {
        todo!()
    }
}
