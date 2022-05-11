//!
//! This module defines a manager for holding multiple instances of mock servers.
//!

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use pact_models::pact::Pact;
use rustls::ServerConfig;
use tracing::debug;

use crate::mock_server::{MockServer, MockServerConfig};

struct ServerEntry {
  mock_server: Arc<Mutex<MockServer>>,
  join_handle: tokio::task::JoinHandle<()>,
}

/// Struct to represent many mock servers running in a background thread
pub struct ServerManager {
    runtime: tokio::runtime::Runtime,
    mock_servers: BTreeMap<String, ServerEntry>,
}

impl ServerManager {
  /// Construct a new ServerManager for scheduling several instances of mock servers
  /// on one tokio runtime.
  pub fn new() -> ServerManager {
    ServerManager {
      runtime: tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap(),
      mock_servers: BTreeMap::new()
    }
  }

    /// Start a new server on the runtime
    pub fn start_mock_server_with_addr(
      &mut self,
      id: String,
      pact: Box<dyn Pact + Send + Sync>,
      addr: SocketAddr,
      config: MockServerConfig
    ) -> Result<SocketAddr, String> {
      let (mock_server, future) =
        self.runtime.block_on(MockServer::new(id.clone(), pact, addr, config))?;

      let port = { mock_server.lock().unwrap().port.clone() };
      self.mock_servers.insert(
        id,
        ServerEntry {
          mock_server,
          join_handle: self.runtime.spawn(future),
        },
      );

      match port {
        Some(port) => Ok(SocketAddr::new(addr.ip(), port)),
        None => Ok(addr)
      }
    }

    /// Start a new TLS server on the runtime
    pub fn start_tls_mock_server_with_addr(
      &mut self,
      id: String,
      pact: Box<dyn Pact>,
      addr: SocketAddr,
      tls_config: &ServerConfig,
      config: MockServerConfig
    ) -> Result<SocketAddr, String> {
      let (mock_server, future) =
        self.runtime.block_on(MockServer::new_tls(id.clone(), pact, addr, tls_config, config))?;

      let port = { mock_server.lock().unwrap().port.clone() };
      self.mock_servers.insert(
        id,
        ServerEntry {
          mock_server,
          join_handle: self.runtime.spawn(future),
        }
      );

      match port {
        Some(port) => Ok(SocketAddr::new(addr.ip(), port)),
        None => Ok(addr)
      }
    }

    /// Start a new server on the runtime
    pub fn start_mock_server(
      &mut self,
      id: String,
      pact: Box<dyn Pact + Send + Sync>,
      port: u16,
      config: MockServerConfig
    ) -> Result<u16, String> {
        self.start_mock_server_with_addr(id, pact, ([0, 0, 0, 0], port as u16).into(), config)
            .map(|addr| addr.port())
    }

  /// Start a new server on the runtime, returning the future
  pub async fn start_mock_server_nonblocking(
    &mut self,
    id: String,
    pact: Box<dyn Pact + Send + Sync>,
    port: u16,
    config: MockServerConfig
  ) -> Result<u16, String> {
    let addr= ([0, 0, 0, 0], port as u16).into();
    let (mock_server, future) = MockServer::new(id.clone(), pact, addr, config).await?;

    let port = {
      mock_server.lock().unwrap().port.clone()
    };
    self.mock_servers.insert(
      id,
      ServerEntry {
        mock_server,
        join_handle: self.runtime.spawn(future),
      },
    );

    port.ok_or_else(|| "Started mock server has no port".to_string())
  }

    /// Start a new TLS server on the runtime
    pub fn start_tls_mock_server(
      &mut self,
      id: String,
      pact: Box<dyn Pact>,
      port: u16,
      tls: &ServerConfig,
      config: MockServerConfig
    ) -> Result<u16, String> {
        self.start_tls_mock_server_with_addr(id, pact, ([0, 0, 0, 0], port as u16).into(), tls, config)
          .map(|addr| addr.port())
    }

    /// Shut down a server by its id
    pub fn shutdown_mock_server_by_id(&mut self, id: String) -> bool {
      match self.mock_servers.remove(&id) {
        Some(entry) => {
          let mut ms = entry.mock_server.lock().unwrap();
          debug!("Shutting down mock server with ID {} - {:?}", id, ms.metrics);
          match ms.shutdown() {
            Ok(()) => {
              self.runtime.block_on(entry.join_handle).unwrap();
              true
            }
            Err(_) => false,
          }
        },
        None => false,
      }
    }

    /// Shut down a server by its local port number
    pub fn shutdown_mock_server_by_port(&mut self, port: u16) -> bool {
      debug!("Shutting down mock server with port {}", port);
      let result = self
        .mock_servers
        .iter()
        .find(|(_id, entry)| entry.mock_server.lock().unwrap().port.unwrap_or_default() == port)
        .map(|(_id, entry)| entry.mock_server.lock().unwrap().id.clone());

      if let Some(id) = result {
        if let Some(entry) = self.mock_servers.remove(&id) {
          let mut ms = entry.mock_server.lock().unwrap();
          debug!("Shutting down mock server with port {} - {:?}", port, ms.metrics);
          return match ms.shutdown() {
            Ok(()) => {
              self.runtime.block_on(entry.join_handle).unwrap();
              true
            }
            Err(_) => false,
          };
        }
      }

      false
    }

    /// Find mock server by id, and map it using supplied function if found
    pub fn find_mock_server_by_id<R>(
      &self,
      id: &String,
      f: &dyn Fn(&MockServer) -> R,
    ) -> Option<R> {
      match self.mock_servers.get(id) {
        Some(entry) => Some(f(&entry.mock_server.lock().unwrap())),
        None => None,
      }
    }

    /// Find a mock server by port number and apply a mutating operation on it if successful
    pub fn find_mock_server_by_port_mut<R>(
      &mut self,
      port: u16,
      f: &dyn Fn(&mut MockServer) -> R,
    ) -> Option<R> {
      match self
        .mock_servers
        .iter_mut()
        .find(|(_id, entry)| entry.mock_server.lock().unwrap().port.unwrap_or_default() == port)
      {
        Some((_id, entry)) => Some(f(&mut entry.mock_server.lock().unwrap())),
        None => None,
      }
    }

    /// Map all the running mock servers
    pub fn map_mock_servers<R>(&self, f: &dyn Fn(&MockServer) -> R) -> Vec<R> {
      let mut results = vec![];
      for (_id_, entry) in self.mock_servers.iter() {
        results.push(f(&entry.mock_server.lock().unwrap()));
      }
      return results;
    }
}

#[cfg(test)]
mod tests {
  use std::{thread, time};
  use std::net::TcpStream;

  use env_logger;
  use pact_models::sync_pact::RequestResponsePact;

  use super::*;

  #[test]
    #[cfg(not(target_os = "windows"))]
    fn manager_should_start_and_shutdown_mock_server() {
        let _ = env_logger::builder().is_test(true).try_init();
        let mut manager = ServerManager::new();
        let start_result = manager.start_mock_server("foobar".into(),
                                                     RequestResponsePact::default().boxed(),
                                                     0, MockServerConfig::default());

        assert!(start_result.is_ok());
        let server_port = start_result.unwrap();

        // Server should be up
        assert!(TcpStream::connect(("127.0.0.1", server_port)).is_ok());

        // Should be able to read matches without blocking
        let matches =
            manager.find_mock_server_by_port_mut(server_port, &|mock_server| mock_server.matches());
        assert_eq!(matches, Some(vec![]));

        let stopped = manager.shutdown_mock_server_by_port(server_port);
        assert!(stopped);

        // The tokio runtime is now out of tasks
        drop(manager);

        let millis = time::Duration::from_millis(100);
        thread::sleep(millis);

        // Server should be down
        assert!(TcpStream::connect(("127.0.0.1", server_port)).is_err());
    }
}
