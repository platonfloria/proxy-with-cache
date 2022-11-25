#![deny(warnings)]

use std::convert::Infallible;

use hyper::client::HttpConnector;
use hyper::service::{make_service_fn, service_fn};
use hyper::{Body, Error, Request, Response, Server, Client, StatusCode};
use hyper_tls::HttpsConnector;


async fn proxy(client: Client<HttpsConnector<HttpConnector>>, mut req: Request<Body>) -> Result<Response<Body>, Error> {
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
        
            let res = client.request(req).await?;
            println!("{:?}", res);
            Ok(res)
        },
        None => {
            let mut bad_request = Response::default();
            *bad_request.status_mut() = StatusCode::BAD_REQUEST;
            Ok(bad_request)
        }
    }
}

#[tokio::main]
pub async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let addr = ([127, 0, 0, 1], 3000).into();

    let https = HttpsConnector::new();
    let client = Client::builder().build::<_, hyper::Body>(https);

    let make_svc = make_service_fn(move |_| {
        let client = client.clone();
        async { Ok::<_, Infallible>(service_fn(move |req| proxy(client.clone(), req))) }
    });

    let server = Server::bind(&addr).serve(make_svc);

    println!("Listening on http://{}", addr);

    server.await?;

    Ok(())
}