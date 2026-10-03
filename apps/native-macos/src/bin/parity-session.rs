//! Real-server transport/read-model runner. This does not certify GPUI interaction parity.
use serde::Deserialize;
use serde_json::{Value, json};
use std::io::{self, Read};
use t3code_native_macos::{
    connection::{self, Update},
    shell::Shell,
    thread::ThreadDetail,
};

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "kebab-case")]
enum Action {
    Dispatch { command: Value },
    Call { tag: String, payload: Value },
    Select { thread_id: String },
}

struct Projection {
    shell: Shell,
    detail: ThreadDetail,
    selected: Option<String>,
}

impl Projection {
    async fn next(
        &mut self,
        updates: &async_channel::Receiver<Update>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        match updates.recv().await? {
            Update::Resynchronizing => {
                self.shell = Shell::default();
                self.detail = ThreadDetail::default();
            }
            Update::Shell(item) => self.shell.apply(item)?,
            Update::Thread { thread_id, item } => {
                if self.selected.as_ref() == Some(&thread_id) {
                    self.detail.apply(&thread_id, item)?;
                }
            }
            Update::Status(status) => {
                if status != "Authorizing…" && status != "Loading projects and threads…" {
                    return Err(status.into());
                }
            }
        }
        Ok(())
    }

    fn observation(&self) -> Value {
        json!({"projects":self.shell.projects,"threads":self.shell.threads,"detail":self.detail.thread})
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut text = String::new();
    io::stdin().read_to_string(&mut text)?;
    let actions: Vec<Action> = serde_json::from_str(&text)?;
    let base = std::env::var("T3_NATIVE_SERVER")?;
    let token = std::env::var("T3_NATIVE_ACCESS_TOKEN")?;
    let session = connection::start_session(connection::ConnectionOptions::new(&base, token)?);
    let mut state = Projection {
        shell: Shell::default(),
        detail: ThreadDetail::default(),
        selected: None,
    };
    let work = async {
        while !state.shell.synchronized {
            state.next(&session.updates).await?;
        }
        let mut frames = Vec::new();
        for action in actions {
            match action {
                Action::Dispatch { command } => {
                    let result = session
                        .call("orchestration.dispatchCommand", command)
                        .await?;
                    let sequence = result["sequence"]
                        .as_u64()
                        .ok_or("Dispatch omitted sequence")?;
                    while state.shell.sequence.is_none_or(|n| n < sequence) {
                        state.next(&session.updates).await?;
                    }
                    frames.push(json!({"receipt":result,"state":state.observation()}));
                }
                Action::Call { tag, payload } => {
                    frames.push(json!({"result":session.call(&tag,payload).await?}))
                }
                Action::Select { thread_id } => {
                    state.selected = Some(thread_id.clone());
                    state.detail = ThreadDetail::default();
                    session
                        .commands
                        .send(connection::Command::SelectThread(Some(thread_id)))
                        .await?;
                    while !state.detail.synchronized {
                        state.next(&session.updates).await?;
                    }
                    frames.push(json!({"state":state.observation()}));
                }
            }
        }
        Ok::<_, Box<dyn std::error::Error>>(frames)
    };
    // Only a failure guard: observable receipts/markers drive every successful wait.
    let frames = tokio::time::timeout(std::time::Duration::from_secs(45), work).await??;
    println!(
        "{}",
        json!({"scope":"native transport and projection, not GUI parity","frames":frames})
    );
    Ok(())
}
