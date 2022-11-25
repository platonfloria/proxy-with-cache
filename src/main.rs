#![deny(warnings)]

use std::collections::{hash_map::DefaultHasher, HashMap};
use std::convert::Infallible;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use hyper::body::Bytes;
use hyper::client::HttpConnector;
use hyper::http::HeaderValue;
use hyper::service::{make_service_fn, service_fn};
use hyper::{Body, Error, Request, Response, Server, Client, StatusCode, Version, HeaderMap};
use hyper_tls::HttpsConnector;
use tokio::sync::Mutex;


const TTL: Duration = Duration::new(30, 0);


struct CachedResponse {
    status: StatusCode,
    version: Version,
    headers: HeaderMap<HeaderValue>,
    body: Bytes,
    expiry: SystemTime,
}

struct Controller {
    client: Client<HttpsConnector<HttpConnector>>,
    cache: Mutex<HashMap<u64, CachedResponse>>,
}

impl Controller {
    fn new() -> Controller {
        let https = HttpsConnector::new();

        Controller {
            client: Client::builder().build::<_, hyper::Body>(https),
            cache: Mutex::new(HashMap::new()),
        }
    }

    pub async fn process(self: Arc<Self>, req: Request<Body>) -> Result<Response<Body>, Error> {
        let req_hash = self.calculate_hash(&format!("{:?}", req));

        let response = {
            let cache = self.cache.lock().await;
            match cache.get(&req_hash) {
                Some(cached_response) => {
                    if cached_response.expiry > SystemTime::now() {
                        let mut response = Response::builder()
                            .status(cached_response.status)
                            .version(cached_response.version)
                            .body(Body::from(cached_response.body.clone()))
                            .unwrap();
                        *response.headers_mut() = cached_response.headers.clone();
                        Some(response)
                    } else {
                        None
                    }
                },
                None => None
            }
        };

        let response = match response {
            Some(response) => response,
            None => {
                let response = self.proxy(req).await?;
                println!("{:?}", response);
                let (parts, body) = response.into_parts();
                let body = hyper::body::to_bytes(body).await?;
                let cached = CachedResponse {
                    status: parts.status.clone(),
                    version: parts.version.clone(),
                    headers: parts.headers.clone(),
                    body: body.clone(),
                    expiry: SystemTime::now() + TTL,
                };
                self.cache.lock().await.insert(req_hash, cached);
                Response::from_parts(parts, Body::from(body))
            }
        };
        Ok(response)
    }

    async fn proxy(&self, mut req: Request<Body>) -> Result<Response<Body>, Error> {
        match req.headers_mut().remove("Origin") {
            Some(origin_address) => {
                let uri_string = format!(
                    "{}{}",
                    origin_address.to_str().unwrap(),
                    req.uri()
                        .path_and_query()
                        .map(|x| x.as_str())
                        .unwrap_or("/")
                );
                let uri = uri_string.parse().unwrap();
                *req.uri_mut() = uri;
            
                let res = self.client.request(req).await?;
                Ok(res)
            },
            None => {
                let mut bad_request = Response::default();
                *bad_request.status_mut() = StatusCode::BAD_REQUEST;
                Ok(bad_request)
            }
        }
    }

    fn calculate_hash<T: Hash>(&self, t: &T) -> u64 {
        let mut s = DefaultHasher::new();
        t.hash(&mut s);
        s.finish()
    }

    pub async fn clear_expired_cache(&self) -> Result<(), Error> {
        loop {
            let now = SystemTime::now();
            self.cache.lock().await.retain(|_, cached| cached.expiry > now);
            tokio::time::sleep(Duration::new(1, 0)).await;
        }
    }
}

#[tokio::main]
pub async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let addr = ([127, 0, 0, 1], 3000).into();

    let controller = Arc::new(Controller::new());

    let make_svc = {
        let controller = controller.clone();
        make_service_fn(move |_| {
            let controller = controller.clone();
            async { Ok::<_, Infallible>(service_fn(move |req| controller.clone().process(req))) }
        })
    };

    let server = Server::bind(&addr).serve(make_svc);

    println!("Listening on http://{}", addr);

    match tokio::join!(
        server,
        controller.clear_expired_cache()
    ) {
        _ => Ok(())
    }
}