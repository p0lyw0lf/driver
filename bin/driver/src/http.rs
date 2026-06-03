use std::path::PathBuf;
use std::pin::Pin;

use http_body_util::Full;
use hyper::{
    Request, Response,
    body::{Bytes, Incoming},
    server::conn::http1,
    service::Service,
};
use hyper_util::rt::TokioIo;
use tokio::net::{TcpListener, ToSocketAddrs};

use driver_engine::query;
use driver_query_ssg::{
    QueryContext,
    boa::{JsObject, JsValue, RunJs, parse_args},
};

pub fn serve(
    bind_addr: impl ToSocketAddrs,
    root: &QueryContext,
    filename: PathBuf,
    extra_args: Vec<String>,
) -> driver_util::Result<()> {
    let service = QueryService {
        root: root.clone(),
        filename,
        extra_args,
    };
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(serve_async(bind_addr, service))
}

async fn serve_async(
    bind_addr: impl ToSocketAddrs,
    service: QueryService,
) -> driver_util::Result<()> {
    let listener = TcpListener::bind(bind_addr).await?;

    loop {
        let (stream, _) = listener.accept().await?;
        let io = TokioIo::new(stream);
        let service = service.clone();
        tokio::task::spawn(async move {
            let out = http1::Builder::new().serve_connection(io, service).await;
            if let Err(err) = out {
                eprintln!("Error serving connection: {err}");
            }
        });
    }
}

#[derive(Clone)]
struct QueryService {
    root: QueryContext,
    filename: PathBuf,
    extra_args: Vec<String>,
}

impl Service<Request<Incoming>> for QueryService {
    type Response = Response<Full<Bytes>>;
    type Error = driver_util::StdError;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn call(&self, req: Request<Incoming>) -> Self::Future {
        let this = self.clone();
        Box::pin(async move {
            let path = req.uri().path();
            let object = this.run(path).await?;
            let mmap = this.root.load_mmap(&object)?;
            let body = Full::new(Bytes::from_owner(mmap));
            Ok(Response::new(body))
        })
    }
}

impl QueryService {
    async fn run(&self, path: &str) -> driver_util::Result<driver_engine::Object> {
        let key = RunJs {
            file: self.filename.clone(),
            arg: parse_args(
                [path]
                    .into_iter()
                    .chain(self.extra_args.iter().map(std::ops::Deref::deref)),
            ),
        };
        let output = query(&self.root, key).await?.export;
        let JsValue::Store(JsObject { ref object }) = output else {
            return Err(driver_util::Error::new(&format!(
                "expected StoreObject as default export, got {output}"
            )));
        };

        Ok(object.clone())
    }
}
