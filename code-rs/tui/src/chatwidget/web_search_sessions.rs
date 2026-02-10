use super::{tool_cards, ChatWidget, OrderKey};
use super::tool_cards::ToolCardSlot;
use crate::history_cell::{WebSearchSessionCell, WebSearchStatus};
use code_core::protocol::OrderMeta;
use std::collections::HashSet;
use std::mem;
use std::time::Instant;

pub(super) struct WebSearchTracker {
    pub slot: ToolCardSlot,
    pub cell: WebSearchSessionCell,
    pub request_ordinal: u64,
    pub started_at: Instant,
    pub active_calls: HashSet<String>,
}

impl WebSearchTracker {
    fn new(order_key: OrderKey, request_ordinal: u64) -> Self {
        Self {
            slot: ToolCardSlot::new(order_key),
            cell: WebSearchSessionCell::new(),
            request_ordinal,
            started_at: Instant::now(),
            active_calls: HashSet::new(),
        }
    }

    fn card_key(&self) -> String {
        web_search_key(self.request_ordinal)
    }

    fn assign_key(&mut self) {
        let key = self.card_key();
        self.cell.set_signature(Some(key.clone()));
        tool_cards::assign_tool_card_key(&mut self.slot, &mut self.cell, Some(key.clone()));
        self.slot.set_signature(Some(key));
    }

    fn ensure_insert(&mut self, chat: &mut ChatWidget<'_>) {
        self.assign_key();
        tool_cards::ensure_tool_card::<WebSearchSessionCell>(chat, &mut self.slot, &self.cell);
    }

    fn replace(&mut self, chat: &mut ChatWidget<'_>) {
        tool_cards::replace_tool_card::<WebSearchSessionCell>(chat, &mut self.slot, &self.cell);
    }
}

pub(super) fn handle_begin(
    chat: &mut ChatWidget<'_>,
    order: Option<&OrderMeta>,
    call_id: String,
    query: Option<String>,
    order_key: OrderKey,
) {
    let request_ordinal = resolve_request_ordinal(order, order_key.req);
    let key = web_search_key(request_ordinal);

    let (mut tracker, existed) = match chat.tools_state.web_search_sessions.remove(&key) {
        Some(tracker) => (tracker, true),
        None => (WebSearchTracker::new(order_key, request_ordinal), false),
    };

    tracker.slot.set_order_key(order_key);
    if !existed {
        tracker.cell.ensure_started_message();
    }

    if let Some(ref q) = query
        && tracker.cell.set_query(Some(q.clone())) {
            tracker
                .cell
                .record_info(tracker.started_at.elapsed(), format!("Query: \"{q}\""));
        }

    tracker.cell.set_status(WebSearchStatus::Running);
    tracker.active_calls.insert(call_id.clone());

    if existed {
        tracker.replace(chat);
    } else {
        tracker.ensure_insert(chat);
    }

    chat
        .tools_state
        .web_search_sessions
        .insert(key.clone(), tracker);
    chat
        .tools_state
        .web_search_by_call
        .insert(call_id, key.clone());
    chat
        .tools_state
        .web_search_by_order
        .insert(request_ordinal, key);
    chat.bottom_pane.update_status_text("Search".to_string());
    chat.request_redraw();
}

pub(super) fn handle_complete(
    chat: &mut ChatWidget<'_>,
    order: Option<&OrderMeta>,
    call_id: String,
    query: Option<String>,
    order_key: OrderKey,
) {
    let request_ordinal = resolve_request_ordinal(order, order_key.req);
    let fallback_key = web_search_key(request_ordinal);
    let key = chat
        .tools_state
        .web_search_by_call
        .remove(&call_id)
        .or_else(|| order.and_then(|meta| chat.tools_state.web_search_by_order.get(&meta.request_ordinal).cloned()))
        .unwrap_or(fallback_key);

    let (mut tracker, existed) = match chat.tools_state.web_search_sessions.remove(&key) {
        Some(tracker) => (tracker, true),
        None => (WebSearchTracker::new(order_key, request_ordinal), false),
    };

    tracker.slot.set_order_key(order_key);
    tracker.request_ordinal = request_ordinal;
    tracker.active_calls.remove(&call_id);

    if let Some(ref q) = query
        && tracker.cell.set_query(Some(q.clone())) {
            tracker
                .cell
                .record_info(tracker.started_at.elapsed(), format!("Query: \"{q}\""));
        }

    let elapsed = tracker.started_at.elapsed();
    tracker
        .cell
        .record_success(elapsed, "Results ready".to_string());
    tracker.cell.set_duration(Some(elapsed));
    tracker.cell.set_status(WebSearchStatus::Completed);

    if existed {
        tracker.replace(chat);
    } else {
        tracker.ensure_insert(chat);
    }

    if !tracker.active_calls.is_empty() {
        chat
            .tools_state
            .web_search_sessions
            .insert(key.clone(), tracker);
        chat
            .tools_state
            .web_search_by_order
            .insert(request_ordinal, key);
    } else {
        chat
            .tools_state
            .web_search_by_order
            .remove(&request_ordinal);
    }

    chat.bottom_pane.update_status_text("responding".to_string());
    chat.maybe_hide_spinner();
}

pub(super) fn finalize_all_failed(chat: &mut ChatWidget<'_>, message: &str) {
    if chat.tools_state.web_search_sessions.is_empty() {
        return;
    }
    let mut trackers = mem::take(&mut chat.tools_state.web_search_sessions);
    chat.tools_state.web_search_by_call.clear();
    chat.tools_state.web_search_by_order.clear();
    for (_, mut tracker) in trackers.drain() {
        tracker.slot.set_order_key(chat.next_internal_key());
        let elapsed = tracker.started_at.elapsed();
        tracker
            .cell
            .record_error(elapsed, message.to_string());
        tracker.cell.set_status(WebSearchStatus::Failed);
        tracker.replace(chat);
    }
}

pub(super) fn finalize_all_completed(chat: &mut ChatWidget<'_>, message: &str) {
    if chat.tools_state.web_search_sessions.is_empty() {
        return;
    }
    let mut trackers = mem::take(&mut chat.tools_state.web_search_sessions);
    chat.tools_state.web_search_by_call.clear();
    chat.tools_state.web_search_by_order.clear();
    for (_, mut tracker) in trackers.drain() {
        tracker.slot.set_order_key(chat.next_internal_key());
        let elapsed = tracker.started_at.elapsed();
        tracker
            .cell
            .record_success(elapsed, message.to_string());
        tracker.cell.set_status(WebSearchStatus::Completed);
        tracker.replace(chat);
    }
}

fn resolve_request_ordinal(order: Option<&OrderMeta>, fallback: u64) -> u64 {
    order
        .map(|meta| meta.request_ordinal)
        .unwrap_or(fallback)
}

fn web_search_key(request_ordinal: u64) -> String {
    format!("web_search:req:{request_ordinal}")
}
