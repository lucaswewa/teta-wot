//! Serving over TLS with a self-signed certificate: HTTP/2 when the client
//! negotiates it by ALPN, HTTP/1.1 when it doesn't.
#![cfg(feature = "tls")]

use std::sync::Arc;

use bytes::Bytes;
use http_body_util::{BodyExt, Empty};
use hyper::{Request, StatusCode, Version};
use hyper_util::rt::{TokioExecutor, TokioIo};
use teta_wot_core::{DataProperty, Prop, Thing, ThingDefinition};
use teta_wot_server::ThingServer;
use teta_wot_server::tls::rustls::pki_types::{CertificateDer, ServerName};
use teta_wot_server::tls::{TlsListener, rustls, server_config};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;
use tokio_rustls::TlsConnector;
use tokio_rustls::client::TlsStream;

struct Lamp {
    on: Prop<bool>,
}

impl Thing for Lamp {
    fn definition() -> ThingDefinition<Self> {
        ThingDefinition::new("Lamp").property("on", DataProperty::new(|t: &Self| &t.on))
    }
}

/// Serves a lamp over TLS, with a new self-signed certificate for
/// `localhost`. Dropping the sender stops the server.
async fn serve() -> (u16, CertificateDer<'static>, oneshot::Sender<()>) {
    let certified = rcgen::generate_simple_self_signed(vec!["localhost".to_owned()]).unwrap();
    let config = server_config(
        certified.cert.pem().as_bytes(),
        certified.signing_key.serialize_pem().as_bytes(),
    )
    .unwrap();
    let tcp = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = tcp.local_addr().unwrap().port();
    let server = ThingServer::builder()
        .thing(
            "lamp",
            Lamp {
                on: Prop::new(true),
            },
        )
        .build()
        .unwrap();
    let (stop, stopped) = oneshot::channel();
    let listener = TlsListener::new(tcp, config).unwrap();
    tokio::spawn(server.serve_with_listener(listener, async move {
        let _ = stopped.await;
    }));
    (port, certified.cert.der().clone(), stop)
}

/// Connects, trusting only `certificate`, and offering one ALPN protocol.
async fn connect(
    port: u16,
    certificate: CertificateDer<'static>,
    alpn: &[u8],
) -> TlsStream<TcpStream> {
    let mut roots = rustls::RootCertStore::empty();
    roots.add(certificate).unwrap();
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_root_certificates(roots)
        .with_no_client_auth();
    config.alpn_protocols = vec![alpn.to_vec()];
    let tcp = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let name = ServerName::try_from("localhost").unwrap();
    TlsConnector::from(Arc::new(config))
        .connect(name, tcp)
        .await
        .unwrap()
}

#[tokio::test]
async fn http2_is_negotiated_and_carries_many_requests_on_one_connection() {
    let (port, certificate, _stop) = serve().await;
    let tls = connect(port, certificate, b"h2").await;
    assert_eq!(tls.get_ref().1.alpn_protocol(), Some(&b"h2"[..]));
    let (sender, connection) =
        hyper::client::conn::http2::handshake(TokioExecutor::new(), TokioIo::new(tls))
            .await
            .unwrap();
    tokio::spawn(connection);
    let requests = (0..20).map(|_| {
        let mut sender = sender.clone();
        async move {
            let uri = format!("https://localhost:{port}/lamp/on");
            let request = Request::get(uri).body(Empty::<Bytes>::new()).unwrap();
            let response = sender.send_request(request).await.unwrap();
            let (version, status) = (response.version(), response.status());
            let body = response.into_body().collect().await.unwrap().to_bytes();
            (version, status, body)
        }
    });
    for (version, status, body) in futures_util::future::join_all(requests).await {
        assert_eq!((version, status), (Version::HTTP_2, StatusCode::OK));
        assert_eq!(&body[..], b"true");
    }
}

#[tokio::test]
async fn http1_is_served_over_tls_to_clients_that_ask_for_it() {
    let (port, certificate, _stop) = serve().await;
    let tls = connect(port, certificate, b"http/1.1").await;
    assert_eq!(tls.get_ref().1.alpn_protocol(), Some(&b"http/1.1"[..]));
    let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(tls))
        .await
        .unwrap();
    tokio::spawn(connection);
    let request = Request::get("/lamp/on")
        .header("host", "localhost")
        .body(Empty::<Bytes>::new())
        .unwrap();
    let response = sender.send_request(request).await.unwrap();
    assert_eq!(
        (response.version(), response.status()),
        (Version::HTTP_11, StatusCode::OK)
    );
}
