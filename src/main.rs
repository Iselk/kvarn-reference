use kvarn::prelude::*;
use kvarn_extensions;

#[cfg_attr(feature = "mt", tokio::main)]
#[cfg_attr(not(feature = "mt"), tokio::main(flavor = "current_thread"))]
async fn main() {
    let env_log = env_logger::Env::default().default_filter_or("rustls=off,warn");
    env_logger::Builder::from_env(env_log).init();

    // Mount all extensions to server
    let mut icelk_extensions = kvarn_extensions::new();

    let times_called = Arc::new(threading::atomic::AtomicUsize::new(0));
    icelk_extensions.add_prepare_single(
        "/test".to_string(),
        prepare!(request, host, path, addr, times_called, {
            let tc = times_called;
            let tc = tc.fetch_add(1, threading::atomic::Ordering::Relaxed);

            let body = build_bytes!(
                b"<h1>Welcome to my site!</h1> You are calling: ",
                request.uri().path().as_bytes(),
                b" for the ",
                tc.to_string().as_bytes(),
                b" time",
            );

            // It must be OK; we haven't changed the response
            let response = Response::new(body);

            (
                response,
                ClientCachePreference::None,
                ServerCachePreference::None,
                CompressPreference::Full,
            )
        }),
    );
    icelk_extensions.add_prepare_single(
        "/throw_500".to_string(),
        prepare!(_req, host, _path, _addr, , {
            utility::default_error_response(StatusCode::INTERNAL_SERVER_ERROR, host).await
        }),
    );
    icelk_extensions.add_prepare_fn(
        Box::new(|req| req.uri().path().starts_with("/capturing/")),
        prepare!(req, host, path, _addr, , {
            let body = build_bytes!(b"!> tmpl standard.html\n\
            [head]\
            [dependencies]\
            [close-head]\
            [navigation]\
            <main style='text-align: center;'><h1>You are visiting: '",
            req.uri().path().as_bytes(),
            b"'.</h1>Well, hope you enjoy <a href='/'>my site</a>!</main>"
        );
            (Response::new(body), ClientCachePreference::Full, ServerCachePreference::None, CompressPreference::Full)
        }),
    );

    #[cfg(feature = "https")]
    let icelk_host = Host::with_http_redirect(
        "icelk.dev",
        "icelk_cert.pem",
        "icelk_pk.pem",
        PathBuf::from("icelk.dev"),
        icelk_extensions,
    );
    let kvarn_extensions = kvarn_extensions::new();

    #[cfg(feature = "https")]
    let kvarn_host = Host::with_http_redirect(
        "kvarn.org",
        "kvarn_cert.pem",
        "kvarn_pk.pem",
        PathBuf::from("kvarn.org"),
        kvarn_extensions,
    );
    #[cfg(not(feature = "https"))]
    let kvarn_host =
        Host::no_certification("kvarn.org", PathBuf::from("kvarn.org"), kvarn_extensions);

    #[cfg(feature = "https")]
    let hosts = HostData::builder(icelk_host).add_host(kvarn_host).build();

    #[cfg(not(feature = "https"))]
    let hosts = HostData::builder(kvarn_host).build();

    #[cfg(not(feature = "high_ports"))]
    let http_port = 80;
    #[cfg(all(not(feature = "high_ports"), feature = "https"))]
    let https_port = 443;
    #[cfg(feature = "high_ports")]
    let http_port = 8080;
    #[cfg(all(feature = "high_ports", feature = "https"))]
    let https_port = 8443;

    let mut ports = Vec::with_capacity(2);

    ports.push(kvarn::HostDescriptor::new(
        http_port,
        Arc::clone(&hosts),
        #[cfg(feature = "https")]
        None,
    ));

    #[cfg(feature = "https")]
    if hosts.has_secure() {
        let mut config = HostData::make_config(&hosts);
        config.alpn_protocols = kvarn::alpn();
        let config = Arc::new(config);
        ports.push(kvarn::HostDescriptor::new(
            https_port,
            Arc::clone(&hosts),
            Some(config),
        ));
    }

    let server = Config::new(ports);

    #[cfg(feature = "interactive")]
    tokio::spawn(async move { server.run().await });
    #[cfg(not(feature = "interactive"))]
    server.run().await;

    #[cfg(feature = "interactive")]
    {
        use http::uri::Uri;
        // Start `kvarn_chute`
        match std::process::Command::new("kvarn_chute").arg(".").spawn() {
            Ok(_child) => println!("Successfully started 'kvarn_chute!'"),
            Err(_) => eprintln!("Failed to start 'kvarn_chute'."),
        }

        use tokio::io::AsyncBufReadExt;
        // Commands in console
        for line in tokio::io::BufReader::new(tokio::io::stdin())
            .lines()
            .next_line()
            .await
        {
            if let Some(line) = line {
                let mut words = line.split(" ");
                if let Some(command) = words.next() {
                    match command {
                        "fcc" => {
                            // File cache clear
                            match hosts
                                .clear_file_in_cache(&Path::new(words.next().unwrap_or(&"")))
                                .await
                            {
                                true => println!("Removed item from cache!"),
                                false => println!("No item to remove"),
                            }
                        }
                        "rcc" => {
                            // Response cache clear
                            let host = match words.next() {
                                Some(word) => word,
                                None => {
                                    println!("Please enter a host to clear cache in.");
                                    continue;
                                }
                            };
                            let uri = match Uri::builder()
                                .path_and_query(words.next().unwrap_or(&""))
                                .build()
                            {
                                Ok(uri) => uri,
                                Err(..) => {
                                    eprintln!("Failed to format path");
                                    continue;
                                }
                            };
                            let (cleared, found) = hosts.clear_page(host, &uri).await;

                            if !found {
                                println!("Did not found host to remove cached item from. Use 'default' or an empty string (e.g. '') for the default host.");
                            } else {
                                if !cleared {
                                    println!("Did not remove any cached response.");
                                } else {
                                    println!("Cleared a cached response.");
                                }
                            }
                        }
                        "cfc" => {
                            hosts.clear_file_caches().await;
                            println!("Cleared file system cache!");
                        }
                        "crc" => {
                            hosts.clear_response_caches().await;
                            println!("Cleared whole response cache.",);
                        }
                        "cc" => {
                            hosts.clear_response_caches().await;
                            hosts.clear_file_caches().await;
                            println!("Cleared all caches!");
                        }
                        _ => {
                            eprintln!("Unknown command!");
                        }
                    }
                }
            };
        }
    }
}
