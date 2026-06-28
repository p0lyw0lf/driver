use wasmtime::component::{Accessor, HasData, Resource, bindgen};

bindgen!({
    world: "guest",
});
use polywolf::driver::boundary::*;

struct State;
struct Environment;

impl HasData for Environment {
    type Data<'a> = &'a mut State;
}

impl HostBlob for Environment {
    fn store(&mut self, bytes: Vec<u8>) -> Resource<Blob> {
        todo!()
    }

    fn store_string(&mut self, s: String) -> Resource<Blob> {
        todo!()
    }

    fn unstore(&mut self, this: Resource<Blob>) -> Vec<u8> {
        todo!()
    }

    fn unstore_string(&mut self, this: Resource<Blob>) -> String {
        todo!()
    }

    fn hash(&mut self, this: Resource<Blob>) -> String {
        todo!()
    }

    fn drop(&mut self, rep: Resource<Blob>) -> wasmtime::Result<()> {
        todo!()
    }
}

impl HostValue for Environment {
    fn kind(&mut self, this: Resource<Value>) -> ValueKind {
        todo!()
    }

    fn as_boolean(&mut self, this: Resource<Value>) -> Option<bool> {
        todo!()
    }

    fn as_int(&mut self, this: Resource<Value>) -> Option<i32> {
        todo!()
    }

    fn as_str(&mut self, this: Resource<Value>) -> Option<String> {
        todo!()
    }

    fn as_array(&mut self, this: Resource<Value>) -> Option<Vec<Resource<Value>>> {
        todo!()
    }

    fn as_object(&mut self, this: Resource<Value>) -> Option<Vec<(String, Resource<Value>)>> {
        todo!()
    }

    fn as_blob(&mut self, this: Resource<Value>) -> Option<Resource<Blob>> {
        todo!()
    }

    fn drop(&mut self, rep: Resource<Value>) -> wasmtime::Result<()> {
        todo!()
    }

    fn of_boolean(&mut self, b: bool) -> Resource<Value> {
        todo!()
    }

    fn of_int(&mut self, i: i32) -> Resource<Value> {
        todo!()
    }

    fn of_str(&mut self, s: String) -> Resource<Value> {
        todo!()
    }

    fn of_array(&mut self, arr: Vec<Resource<Value>>) -> Resource<Value> {
        todo!()
    }

    fn of_object(&mut self, obj: Vec<(String, Resource<Value>)>) -> Resource<Value> {
        todo!()
    }

    fn of_blob(&mut self, this: Resource<Value>, b: Resource<Blob>) -> Resource<Value> {
        todo!()
    }
}

impl Host for Environment {
    fn print(&mut self, text: String) -> () {
        todo!()
    }

    fn slugify(&mut self, s: String) -> String {
        todo!()
    }

    fn write_output(&mut self, path: Path, contents: Resource<Blob>) -> Option<Error> {
        todo!()
    }
}

impl<T> HostWithStore<T> for Environment {
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
