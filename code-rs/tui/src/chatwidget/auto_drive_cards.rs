use super::{tool_cards, ChatWidget, OrderKey};
use super::tool_cards::ToolCardSlot;
use crate::history_cell::{AutoDriveActionKind, AutoDriveCardCell, AutoDriveStatus};

pub(super) struct AutoDriveTracker {
    pub slot: ToolCardSlot,
    pub cell: AutoDriveCardCell,
    pub session_id: u64,
    pub request_ordinal: u64,
    pub active: bool,
}

impl AutoDriveTracker {
    fn new(order_key: OrderKey, session_id: u64, request_ordinal: u64, goal: Option<String>) -> Self {
        Self {
            slot: ToolCardSlot::new(order_key),
            cell: AutoDriveCardCell::new(goal),
            session_id,
            request_ordinal,
            active: true,
        }
    }

    fn card_key(&self) -> String {
        format!("auto_drive:{}", self.session_id)
    }

    fn assign_key(&mut self) {
        let signature = Some(self.card_key());
        self.cell.set_signature(signature.clone());
        tool_cards::assign_tool_card_key(&mut self.slot, &mut self.cell, signature.clone());
        if let Some(sig) = signature {
            self.slot.set_signature(Some(sig));
        }
    }

    fn ensure_insert(&mut self, chat: &mut ChatWidget<'_>) {
        self.assign_key();
        tool_cards::ensure_tool_card::<AutoDriveCardCell>(chat, &mut self.slot, &self.cell);
    }

    fn replace(&mut self, chat: &mut ChatWidget<'_>) {
        tool_cards::replace_tool_card::<AutoDriveCardCell>(chat, &mut self.slot, &self.cell);
    }
}

pub(super) fn start_session(
    chat: &mut ChatWidget<'_>,
    order_key: OrderKey,
    goal: Option<String>,
) {
    let request_ordinal = order_key.req;

    if let Some(mut tracker) = chat.tools_state.auto_drive_tracker.take()
        && tracker.request_ordinal == request_ordinal {
            tracker.slot.set_order_key(order_key);
            tracker.replace(chat);
            chat.tools_state.auto_drive_tracker = Some(tracker);
            return;
        }

    let session_id = chat.auto_drive_card_sequence;
    chat.auto_drive_card_sequence = chat.auto_drive_card_sequence.wrapping_add(1);

    let mut tracker = AutoDriveTracker::new(order_key, session_id, request_ordinal, goal);
    tracker.ensure_insert(chat);
    chat.tools_state.auto_drive_tracker = Some(tracker);
}

pub(super) fn record_action(
    chat: &mut ChatWidget<'_>,
    order_key: OrderKey,
    text: impl Into<String>,
    kind: AutoDriveActionKind,
) {
    if let Some(mut tracker) = chat.tools_state.auto_drive_tracker.take() {
        tracker.request_ordinal = order_key.req;
        tracker.slot.set_order_key(order_key);
        tracker.cell.push_action(text, kind);
        tracker.replace(chat);
        chat.tools_state.auto_drive_tracker = Some(tracker);
    }
}

pub(super) fn update_goal(
    chat: &mut ChatWidget<'_>,
    order_key: OrderKey,
    goal: Option<String>,
) {
    if let Some(mut tracker) = chat.tools_state.auto_drive_tracker.take() {
        tracker.request_ordinal = order_key.req;
        tracker.slot.set_order_key(order_key);
        tracker.cell.set_goal(goal);
        tracker.replace(chat);
        chat.tools_state.auto_drive_tracker = Some(tracker);
    }
}

pub(super) fn set_status(chat: &mut ChatWidget<'_>, order_key: OrderKey, status: AutoDriveStatus) {
    if let Some(mut tracker) = chat.tools_state.auto_drive_tracker.take() {
        tracker.request_ordinal = order_key.req;
        tracker.slot.set_order_key(order_key);
        tracker.cell.set_status(status);
        tracker.replace(chat);
        chat.tools_state.auto_drive_tracker = Some(tracker);
    }
}

pub(super) fn finalize(
    chat: &mut ChatWidget<'_>,
    order_key: OrderKey,
    message: Option<String>,
    status: AutoDriveStatus,
    action_kind: AutoDriveActionKind,
    completion_message: Option<String>,
) {
    if let Some(mut tracker) = chat.tools_state.auto_drive_tracker.take() {
        tracker.request_ordinal = order_key.req;
        tracker.slot.set_order_key(order_key);
        if let Some(msg) = message {
            tracker.cell.push_action(msg, action_kind);
        }
        tracker.cell.set_completion_message(completion_message);
        tracker.cell.set_status(status);
        tracker.replace(chat);
        tracker.active = false;
        chat.tools_state.auto_drive_tracker = Some(tracker);
    }
}

pub(super) fn start_celebration(
    chat: &mut ChatWidget<'_>,
    message: Option<String>,
) -> bool {
    if let Some(mut tracker) = chat.tools_state.auto_drive_tracker.take() {
        tracker.cell.start_celebration(message);
        tracker.replace(chat);
        chat.tools_state.auto_drive_tracker = Some(tracker);
        return true;
    }
    false
}

pub(super) fn stop_celebration(chat: &mut ChatWidget<'_>) -> bool {
    if let Some(mut tracker) = chat.tools_state.auto_drive_tracker.take() {
        tracker.cell.stop_celebration();
        tracker.replace(chat);
        chat.tools_state.auto_drive_tracker = Some(tracker);
        return true;
    }
    false
}

pub(super) fn update_completion_message(
    chat: &mut ChatWidget<'_>,
    message: Option<String>,
) -> bool {
    if let Some(mut tracker) = chat.tools_state.auto_drive_tracker.take() {
        tracker.cell.set_completion_message(message);
        tracker.replace(chat);
        chat.tools_state.auto_drive_tracker = Some(tracker);
        return true;
    }
    false
}

pub(super) fn clear(chat: &mut ChatWidget<'_>) {
    chat.tools_state.auto_drive_tracker = None;
}
