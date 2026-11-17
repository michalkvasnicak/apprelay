//! Scriptable VPN agent for API integration tests, without root or WireGuard.

use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

use tokio::{sync::mpsc, task::JoinHandle};

pub use super::protocol::Request as AgentRequest;
use super::{
    Client, VpnStatus,
    protocol::{self, Response},
};

struct State {
    status: VpnStatus,
    responses: VecDeque<Response>,
    requests: Vec<AgentRequest>,
}

/// Uses the real client's framed JSON protocol over in-memory connections.
/// Requests consume queued responses, or return the configured status when the
/// queue is empty. It deliberately does not simulate WireGuard's peer state.
/// Dropping this handle stops the agent, including for cloned clients.
pub struct FakeAgent {
    client: Client,
    state: Arc<Mutex<State>>,
    task: JoinHandle<()>,
}

impl FakeAgent {
    /// Must be called within a Tokio runtime.
    pub fn new(status: VpnStatus) -> Self {
        let state = Arc::new(Mutex::new(State {
            status,
            responses: VecDeque::new(),
            requests: Vec::new(),
        }));
        let (sender, mut connections) = mpsc::channel::<tokio::io::DuplexStream>(16);
        let agent_state = Arc::clone(&state);
        let task = tokio::spawn(async move {
            while let Some(mut stream) = connections.recv().await {
                let Ok(request) = protocol::receive::<AgentRequest>(&mut stream).await else {
                    continue;
                };
                let response = {
                    let mut state = agent_state.lock().expect("fake VPN agent state poisoned");
                    state.requests.push(request);
                    state
                        .responses
                        .pop_front()
                        .unwrap_or_else(|| Response::Status {
                            status: state.status.clone(),
                        })
                };
                // A cancelled request may close its connection before the reply.
                let _ = protocol::send(&mut stream, &response).await;
            }
        });
        Self {
            client: Client::fake(sender),
            state,
            task,
        }
    }

    pub fn client(&self) -> Client {
        self.client.clone()
    }

    /// Changes the fallback response without consuming or clearing queued replies.
    pub fn set_status(&self, status: VpnStatus) {
        self.state
            .lock()
            .expect("fake VPN agent state poisoned")
            .status = status;
    }

    /// Queues a one-shot reply in request arrival order, including agent errors.
    pub fn enqueue_response(&self, response: Result<VpnStatus, String>) {
        let response = match response {
            Ok(status) => Response::Status { status },
            Err(message) => Response::Error { message },
        };
        self.state
            .lock()
            .expect("fake VPN agent state poisoned")
            .responses
            .push_back(response);
    }

    /// Returns all received requests in arrival order, including failed requests.
    pub fn requests(&self) -> Vec<AgentRequest> {
        self.state
            .lock()
            .expect("fake VPN agent state poisoned")
            .requests
            .clone()
    }
}

impl Drop for FakeAgent {
    fn drop(&mut self) {
        self.task.abort();
    }
}
