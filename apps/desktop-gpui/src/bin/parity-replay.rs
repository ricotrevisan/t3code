//! Headless production-reducer adapter for differential tests, not a UI parity runner.
use serde::Deserialize;
use serde_json::{Value, json};
use std::io::{self, Read};
use t3code_desktop_gpui::thread::apply_event;

#[derive(Deserialize)]
struct Input {
    thread: Value,
    events: Vec<Value>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut text = String::new();
    io::stdin().read_to_string(&mut text)?;
    let input: Input = serde_json::from_str(&text)?;
    let mut thread = Some(input.thread);
    let mut frames = Vec::new();
    for event in input.events {
        thread = apply_event(
            thread.as_ref().ok_or("Event after thread deletion")?,
            &event,
        )?;
        frames.push(thread.clone());
    }
    println!("{}", json!({"frames":frames}));
    Ok(())
}
