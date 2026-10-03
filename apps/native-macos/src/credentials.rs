//! A single blocking worker serializes Keychain writes and deletion. Forget always follows
//! a previously requested remember, even if a system authorization prompt delays that write.
use crate::auth;
use security_framework::passwords::{
    delete_generic_password, get_generic_password, set_generic_password,
};

const SERVICE: &str = "codes.t3.native.access-token";
type Reply = async_channel::Sender<Result<Option<String>, &'static str>>;
pub enum Operation {
    Load,
    Remember(String),
    Forget,
}
struct Command {
    origin: String,
    operation: Operation,
    reply: Reply,
}

trait Store: Send + 'static {
    fn apply(&mut self, origin: &str, operation: Operation)
    -> Result<Option<String>, &'static str>;
}
struct Keychain;
impl Store for Keychain {
    fn apply(
        &mut self,
        origin: &str,
        operation: Operation,
    ) -> Result<Option<String>, &'static str> {
        match operation {
            Operation::Load => match get_generic_password(SERVICE, origin) {
                Ok(bytes) => String::from_utf8(bytes)
                    .map(Some)
                    .map_err(|_| "Keychain credential is invalid"),
                Err(error) if error.code() == -25300 => Ok(None),
                Err(_) => Err("Could not read credential from Keychain"),
            },
            Operation::Remember(token) => {
                set_generic_password(SERVICE, origin, token.as_bytes())
                    .map_err(|_| "Could not save credential in Keychain")?;
                Ok(None)
            }
            Operation::Forget => match delete_generic_password(SERVICE, origin) {
                Ok(()) => Ok(None),
                Err(error) if error.code() == -25300 => Ok(None),
                Err(_) => Err("Could not remove credential from Keychain"),
            },
        }
    }
}

pub struct Vault {
    commands: async_channel::Sender<Command>,
}
impl Default for Vault {
    fn default() -> Self {
        Self::worker(Keychain)
    }
}
impl Vault {
    fn worker(mut store: impl Store) -> Self {
        let (commands, requests) = async_channel::bounded::<Command>(16);
        std::thread::spawn(move || {
            while let Ok(command) = requests.recv_blocking() {
                let result = store.apply(&command.origin, command.operation);
                let _ = command.reply.send_blocking(result);
            }
        });
        Self { commands }
    }
    pub fn request(
        &self,
        server: &str,
        operation: Operation,
    ) -> Result<async_channel::Receiver<Result<Option<String>, &'static str>>, &'static str> {
        let origin = auth::origin(server)?;
        if let Operation::Remember(token) = &operation
            && token.trim().is_empty()
        {
            return Err("Cannot remember an empty credential");
        }
        let (reply, result) = async_channel::bounded(1);
        self.commands
            .try_send(Command {
                origin,
                operation,
                reply,
            })
            .map_err(|_| "Credential worker is busy or unavailable")?;
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn forget_serializes_after_delayed_save_and_never_touches_another_origin() {
        struct Memory {
            secrets: std::collections::HashMap<String, String>,
            entered: std::sync::mpsc::Sender<()>,
            resume: std::sync::mpsc::Receiver<()>,
        }
        impl Store for Memory {
            fn apply(
                &mut self,
                origin: &str,
                operation: Operation,
            ) -> Result<Option<String>, &'static str> {
                match operation {
                    Operation::Remember(token) => {
                        self.entered.send(()).unwrap();
                        self.resume.recv().unwrap();
                        self.secrets.insert(origin.into(), token);
                        Ok(None)
                    }
                    Operation::Forget => {
                        self.secrets.remove(origin);
                        Ok(None)
                    }
                    Operation::Load => Ok(self.secrets.get(origin).cloned()),
                }
            }
        }
        let (entered, receipt) = std::sync::mpsc::channel();
        let (resume, wait) = std::sync::mpsc::channel();
        let vault = Vault::worker(Memory {
            secrets: [("https://other".into(), "other-token".into())].into(),
            entered,
            resume: wait,
        });
        let save = vault
            .request("https://host", Operation::Remember("host-token".into()))
            .unwrap();
        receipt.recv().unwrap();
        let forget = vault.request("https://host", Operation::Forget).unwrap();
        resume.send(()).unwrap();
        assert_eq!(save.recv_blocking().unwrap(), Ok(None));
        assert_eq!(forget.recv_blocking().unwrap(), Ok(None));
        assert_eq!(
            vault
                .request("https://host", Operation::Load)
                .unwrap()
                .recv_blocking()
                .unwrap(),
            Ok(None)
        );
        assert_eq!(
            vault
                .request("https://other", Operation::Load)
                .unwrap()
                .recv_blocking()
                .unwrap(),
            Ok(Some("other-token".into()))
        );
        assert!(
            vault
                .request("https://host", Operation::Remember("".into()))
                .is_err()
        );
        assert!(
            vault
                .request("https://user:password@host", Operation::Load)
                .is_err()
        );
    }
}
