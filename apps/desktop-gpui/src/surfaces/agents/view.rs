//! GPUI adapter for the runtime roster; provider configuration stays in Settings.
use crate::NativeApp;
use gpui_kit::component::{
    ActiveTheme as _, Disableable, Sizable,
    button::{Button, ButtonVariants as _},
};
use gpui_kit::{Context, Entity, Render, Task, Window, div, prelude::*};
use serde_json::{Value, json};
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
};
use t3code_desktop_gpui::{
    connection,
    conversation::{self, ActivityItem, TimelineEntry},
    surfaces::agents::{self, Agent, Status},
};

type ScriptLoad = (u64, Result<Option<Value>, String>);
#[derive(Default)]
pub(crate) struct SurfaceState {
    pub open: RefCell<HashMap<String, bool>>,
    phase_open: RefCell<HashMap<String, (bool, bool)>>,
    pub scripts: HashMap<String, ScriptLoad>,
    script_epoch: u64,
    ticking: Cell<bool>,
    clock_scope: RefCell<Option<(u64, Option<String>)>>,
    timer: RefCell<Option<Task<()>>>,
    roster: RefCell<agents::RosterCache>,
    elapsed: RefCell<HashMap<String, Entity<AgentElapsed>>>,
}
struct AgentElapsed {
    agent: Agent,
    text: String,
}
impl Render for AgentElapsed {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let color = if self.agent.status == Status::Failed {
            cx.theme().danger
        } else if self.agent.status == Status::Completed {
            cx.theme().success
        } else if self.agent.status.active() {
            cx.theme().info
        } else {
            cx.theme().muted_foreground
        };
        div().text_xs().text_color(color).child(self.text.clone())
    }
}
fn elapsed_text(agent: &Agent, now: &str) -> String {
    format!(
        "{} · {}",
        agent.status.label(),
        agent.elapsed(now).unwrap_or_default()
    )
}
impl NativeApp {
    fn agent_surface_key(&self, id: &str) -> String {
        format!(
            "{}:{}:{}:{id}",
            self.environment_scope,
            self.generation,
            self.selected_thread.as_deref().unwrap_or_default()
        )
    }
    fn agent_clock(&self, cx: &mut Context<Self>) {
        let generation = self.generation;
        let thread = self.selected_thread.clone();
        let scope = (generation, thread.clone());
        if self.agents_surface.ticking.get()
            && self.agents_surface.clock_scope.borrow().as_ref() == Some(&scope)
        {
            return;
        }
        self.agents_surface.ticking.set(true);
        *self.agents_surface.clock_scope.borrow_mut() = Some(scope);
        *self.agents_surface.timer.borrow_mut() = Some(cx.spawn(async move |app, cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_secs(1))
                    .await;
                let keep = app
                    .update(cx, |app, cx| {
                        let keep = app.generation == generation
                            && app.selected_thread == thread
                            && app.inspector_open
                            && app.right_tab == crate::RightTab::Agents
                            && app.agents_surface.roster.borrow().live();
                        if !keep {
                            app.agents_surface.ticking.set(false);
                        } else {
                            let now = chrono::Utc::now().to_rfc3339();
                            for label in app.agents_surface.elapsed.borrow().values() {
                                label.update(cx, |label, cx| {
                                    let text = elapsed_text(&label.agent, &now);
                                    if label.text != text {
                                        label.text = text;
                                        cx.notify();
                                    }
                                });
                            }
                        }
                        keep
                    })
                    .unwrap_or(false);
                if !keep {
                    break;
                }
            }
        }));
    }
    pub(crate) fn render_runtime_agents(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let muted = cx.theme().muted_foreground;
        let selected = self
            .detail
            .thread
            .as_ref()
            .filter(|t| t["id"].as_str() == self.selected_thread.as_deref());
        let roster = selected
            .map(|thread| {
                self.agents_surface.roster.borrow_mut().get(
                    self.generation,
                    self.detail.revision,
                    thread,
                )
            })
            .unwrap_or_default();
        let prefix = self.agent_surface_key("");
        self.agents_surface.elapsed.borrow_mut().retain(|key, _| {
            key.starts_with(&prefix) && roster.iter().any(|a| *key == format!("{prefix}{}", a.id))
        });
        let now = chrono::Utc::now().to_rfc3339();
        if roster
            .iter()
            .any(|a| matches!(a.status, Status::Running | Status::Waiting))
        {
            self.agent_clock(cx);
        }
        let mut body = div()
            .id("agents-panel")
            .debug_selector(|| "agents-panel".into())
            .flex()
            .flex_col()
            .gap_2()
            .flex_1()
            .min_h_0();
        if roster.is_empty() {
            return body.child(div().text_sm().child(if selected.is_some() { "No agents yet" } else if self.selected_thread.is_some() { "Thread details are not available yet" } else { "Select a thread to see its agents" }))
                .child(div().text_xs().text_color(muted).child("Subagents and workflows spawned by this thread appear here with status, activity and token usage."));
        }
        let mut rows = div()
            .id("runtime-agent-rows")
            .overflow_y_scroll()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .gap_2();
        for workflow in roster.iter().filter(|a| a.kind == "workflow") {
            let key = self.agent_surface_key(&workflow.id);
            let open = {
                let mut choices = self.agents_surface.open.borrow_mut();
                if choices.len() >= 256 {
                    choices.clear();
                }
                *choices
                    .entry(key.clone())
                    .or_insert(!workflow.status.terminal())
            };
            let members: Vec<_> = roster
                .iter()
                .filter(|a| a.parent.as_deref() == Some(&workflow.id))
                .collect();
            let settled = members.iter().filter(|m| m.status.terminal()).count();
            let tokens = if members.is_empty() {
                workflow.usage["totalTokens"].as_f64().unwrap_or(0.)
            } else {
                members
                    .iter()
                    .map(|m| m.usage["totalTokens"].as_f64().unwrap_or(0.))
                    .sum()
            };
            let title = format!(
                "{} · {settled}/{} settled · {tokens:.0} tok{}",
                workflow.title,
                members.len(),
                workflow
                    .elapsed(&now)
                    .map(|s| format!(" · {s}"))
                    .unwrap_or_default()
            );
            let toggle_key = key.clone();
            let mut group = div().flex().flex_col().gap_1().child(
                Button::new(gpui_kit::SharedString::from(format!(
                    "workflow-{}",
                    workflow.id
                )))
                .debug_selector({
                    let id = workflow.id.clone();
                    move || format!("workflow-{id}")
                })
                .ghost()
                .small()
                .label(format!(
                    "{} {} · {}",
                    if open { "▾" } else { "▸" },
                    title,
                    workflow.status.label()
                ))
                .on_click(cx.listener(move |app, _, _, cx| {
                    app.agents_surface
                        .open
                        .borrow_mut()
                        .insert(toggle_key.clone(), !open);
                    cx.notify();
                })),
            );
            if open {
                group = group.child(self.runtime_agent_row(workflow, &now, cx));
                if let Some(path) = &workflow.script {
                    let id = workflow.id.clone();
                    let path = path.clone();
                    group = group.child(
                        Button::new(gpui_kit::SharedString::from(format!(
                            "workflow-script-{id}"
                        )))
                        .debug_selector({
                            let id = id.clone();
                            move || format!("workflow-script-{id}")
                        })
                        .ghost()
                        .small()
                        .label("View workflow script")
                        .disabled(self.commands.is_none())
                        .on_click(
                            cx.listener(move |app, _, _, cx| app.load_agent_script(&id, &path, cx)),
                        ),
                    );
                    if let Some((_, result)) = self.agents_surface.scripts.get(&key) {
                        let content = match result {
                            Ok(Some(v)) => format!(
                                "{}{}",
                                v["contents"].as_str().unwrap_or_default(),
                                if v["truncated"] == true {
                                    "\n… (truncated)"
                                } else {
                                    ""
                                }
                            ),
                            Ok(None) => "Loading script…".into(),
                            Err(e) => format!("Could not load script: {e}"),
                        };
                        group = group.child(
                            div()
                                .id(gpui_kit::SharedString::from(format!(
                                    "agent-script-{}",
                                    workflow.id
                                )))
                                .max_h(gpui_kit::px(240.))
                                .overflow_scroll()
                                .text_xs()
                                .child(content),
                        );
                        let key = key.clone();
                        group = group.child(
                            Button::new(gpui_kit::SharedString::from(format!(
                                "close-script-{}",
                                workflow.id
                            )))
                            .debug_selector({
                                let id = workflow.id.clone();
                                move || format!("close-script-{id}")
                            })
                            .ghost()
                            .small()
                            .label("Close script")
                            .on_click(cx.listener(
                                move |app, _, _, cx| {
                                    app.agents_surface.scripts.remove(&key);
                                    cx.notify();
                                },
                            )),
                        );
                    }
                }
                let mut phases = workflow.phases.clone();
                if phases.is_empty() {
                    for member in &members {
                        if let Some(i) = member.phase
                            && !phases.iter().any(|p| p.0 == i)
                        {
                            phases.push((
                                i,
                                member
                                    .phase_title
                                    .clone()
                                    .unwrap_or_else(|| format!("Phase {}", i + 1)),
                            ));
                        }
                    }
                    phases.sort_by_key(|p| p.0);
                }
                for (index, title) in &phases {
                    let mut phase_members: Vec<_> = members
                        .iter()
                        .filter(|m| m.phase == Some(*index))
                        .copied()
                        .collect();
                    phase_members.sort_by_key(|m| m.index);
                    let active = phase_members
                        .iter()
                        .filter(|m| m.status.active() || m.status == Status::Idle)
                        .count();
                    let done = phase_members.iter().filter(|m| m.status.terminal()).count();
                    let phase_key = format!("{key}:phase:{index}");
                    let phase_live = active > 0;
                    let phase_open = {
                        let mut choices = self.agents_surface.phase_open.borrow_mut();
                        if choices.len() >= 512 {
                            choices.clear();
                        }
                        let choice = choices
                            .entry(phase_key.clone())
                            .or_insert((phase_live || workflow.status.terminal(), phase_live));
                        if phase_live && !choice.1 {
                            choice.0 = true;
                        }
                        choice.1 = phase_live;
                        choice.0
                    };
                    group = group.child(
                        Button::new(gpui_kit::SharedString::from(format!(
                            "workflow-phase-{}-{index}",
                            workflow.id
                        )))
                        .ghost()
                        .xsmall()
                        .label(format!(
                            "{} {title} · {}",
                            if phase_open { "▾" } else { "▸" },
                            if phase_members.is_empty() {
                                "pending".into()
                            } else if active > 0 {
                                format!("{active} active · {done} done")
                            } else {
                                format!("{done} done")
                            }
                        ))
                        .on_click(cx.listener(move |app, _, _, cx| {
                            app.agents_surface
                                .phase_open
                                .borrow_mut()
                                .insert(phase_key.clone(), (!phase_open, phase_live));
                            cx.notify();
                        })),
                    );
                    if phase_open {
                        for member in phase_members {
                            group = group.child(self.runtime_agent_row(member, &now, cx));
                        }
                    }
                }
                for member in members
                    .iter()
                    .filter(|m| !phases.iter().any(|p| Some(p.0) == m.phase))
                {
                    group = group.child(self.runtime_agent_row(member, &now, cx));
                }
            }
            rows = rows.child(group);
        }
        let direct: Vec<_> = roster
            .iter()
            .filter(|a| {
                a.kind != "workflow"
                    && !roster
                        .iter()
                        .any(|p| p.kind == "workflow" && a.parent.as_deref() == Some(&p.id))
            })
            .collect();
        if !direct.is_empty() {
            rows = rows.child(div().text_xs().text_color(muted).child("Direct spawns"));
        }
        for a in direct {
            rows = rows.child(self.runtime_agent_row(a, &now, cx));
        }
        let (working, idle, settled, tokens) = agents::totals(&roster);
        body = body
            .child(rows)
            .child(div().text_xs().text_color(muted).child(format!(
                "{working} working · {idle} idle · {settled} settled · Σ {tokens:.0} tok"
            )));
        body
    }
    fn runtime_agent_row(
        &self,
        a: &Agent,
        now: &str,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let key = self.agent_surface_key(&a.id);
        let label = {
            let mut labels = self.agents_surface.elapsed.borrow_mut();
            if labels.len() >= 100 && !labels.contains_key(&key) {
                labels.clear();
            }
            let label = labels
                .entry(key)
                .or_insert_with(|| {
                    cx.new(|_| AgentElapsed {
                        agent: a.clone(),
                        text: elapsed_text(a, now),
                    })
                })
                .clone();
            label.update(cx, |label, cx| {
                if label.agent != *a {
                    label.agent = a.clone();
                    label.text = elapsed_text(a, now);
                    cx.notify();
                }
            });
            label
        };
        let activity = a.activity().unwrap_or(a.status.label()).to_owned();
        let id = a.activity_id.clone();
        let mut metadata = a.model.clone().unwrap_or_else(|| "Unknown model".into());
        if let Some(effort) = &a.effort {
            metadata.push_str(&format!(" · {effort}"));
        }
        if let Some(tokens) = a.usage["totalTokens"].as_f64() {
            metadata.push_str(&format!(" · {tokens:.0} tok"));
        } else {
            metadata.push_str(" · — tok");
        }
        if let Some(tools) = a.usage["toolUses"].as_f64() {
            metadata.push_str(&format!(" · {tools:.0} tools"));
        }
        if a.activations > 1 {
            metadata.push_str(&format!(" · run {}", a.activations));
        }
        let mut title = a.title.clone();
        if let Some(role) = &a.role
            && !role.eq_ignore_ascii_case(&a.title)
        {
            title.push_str(&format!(" · {role}"));
        }
        div()
            .id(gpui_kit::SharedString::from(format!(
                "runtime-agent-{}",
                a.id
            )))
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(div().flex_1().min_w_0().text_sm().child(title))
                    .child(label),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(activity),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(metadata),
            )
            .when(!id.is_empty(), |v| {
                v.child(
                    Button::new(gpui_kit::SharedString::from(format!(
                        "agent-timeline-{}",
                        a.id
                    )))
                    .ghost()
                    .xsmall()
                    .label("Show in timeline")
                    .debug_selector({
                        let a = a.id.clone();
                        move || format!("agent-timeline-{a}")
                    })
                    .on_click(cx.listener(move |app, _, _, cx| app.reveal_agent_activity(&id, cx))),
                )
            })
    }
    pub(crate) fn reveal_agent_activity(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(thread) = self
            .detail
            .thread
            .as_ref()
            .filter(|t| t["id"].as_str() == self.selected_thread.as_deref())
        else {
            return;
        };
        // A timeline link is also a way back into a folded turn. Unfold it
        // before resolving the current row index, never navigate a stale index.
        if let Some(group) = conversation::group_turns(thread)
            .into_iter()
            .find(|g| g.activities.iter().any(|a| a["id"] == id))
        {
            self.activity_expanded
                .remove(&format!("fold:{}", group.key));
            self.activity_expanded.insert(group.key, true);
        }
        let rows = conversation::timeline_rows(thread, &self.activity_expanded);
        let target = rows
            .iter()
            .enumerate()
            .find_map(|(i, row)| match &row.entry {
                TimelineEntry::Activity { turn, items, .. }
                    if items.iter().any(|item| match item {
                        ActivityItem::Activity(v) | ActivityItem::Error(v) => v["id"] == id,
                        ActivityItem::Tool(t) => t.activities.iter().any(|v| v["id"] == id),
                    }) =>
                {
                    Some((i, turn.clone()))
                }
                _ => None,
            });
        if let Some((i, turn)) = target {
            self.activity_expanded.insert(turn, true);
            let diff = conversation::diff_rows(&self.conversation_rows, &rows);
            if !diff.replaced.is_empty() || diff.inserted > 0 {
                self.conversation_scroll
                    .splice(diff.replaced, diff.inserted);
            }
            for changed in diff.changed {
                self.conversation_scroll
                    .remeasure_items(changed..changed + 1);
            }
            self.conversation_rows = std::rc::Rc::new(rows);
            self.conversation_scroll.scroll_to_reveal_item(i);
            cx.notify();
        }
    }
    fn load_agent_script(&mut self, id: &str, path: &str, cx: &mut Context<Self>) {
        let Some(commands) = self.commands.clone() else {
            return;
        };
        let Some(thread) = self.selected_thread.clone() else {
            return;
        };
        let key = self.agent_surface_key(id);
        if matches!(self.agents_surface.scripts.get(&key), Some((_, Ok(None)))) {
            return;
        }
        // Bounded presentation cache; generation-scoped keys fence late receipts.
        if self.agents_surface.scripts.len() >= 16 {
            self.agents_surface.scripts.clear();
        }
        self.agents_surface.script_epoch += 1;
        let epoch = self.agents_surface.script_epoch;
        self.agents_surface
            .scripts
            .insert(key.clone(), (epoch, Ok(None)));
        let generation = self.generation;
        let payload = json!({"threadId":thread,"scriptPath":path});
        cx.spawn(async move |app, cx| {
            let result = connection::call(&commands, "orchestration.getWorkflowScript", payload)
                .await
                .and_then(|value| {
                    if value["contents"].is_string() && value["truncated"].is_boolean() {
                        Ok(value)
                    } else {
                        Err("Invalid workflow script response".into())
                    }
                });
            let _ = app.update(cx, |app, cx| {
                if app.generation != generation
                    || !app
                        .agents_surface
                        .scripts
                        .get(&key)
                        .is_some_and(|(current, _)| *current == epoch)
                {
                    return;
                }
                app.agents_surface
                    .scripts
                    .insert(key, (epoch, result.map(Some)));
                // Cache a completed receipt for its captured thread. Navigation does
                // not orphan the loading sentinel or display the script on another thread.
                if app.selected_thread.as_deref() == Some(&thread) {
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }
}
