use super::{ChatWidget, OrderKey};
use crate::history::state::HistoryId;
use crate::history_cell::{
    AgentRunCell,
    CollapsibleReasoningCell,
    HistoryCell,
    HistoryCellType,
};
use std::any::TypeId;

pub(super) struct ToolCardSlot {
    pub order_key: OrderKey,
    pub history_id: Option<HistoryId>,
    pub cell_index: Option<usize>,
    pub cell_key: Option<String>,
    previous_key: Option<String>,
    signature: Option<String>,
    last_order_key: Option<OrderKey>,
}

impl ToolCardSlot {
    pub fn new(order_key: OrderKey) -> Self {
        Self {
            order_key,
            history_id: None,
            cell_index: None,
            cell_key: None,
            previous_key: None,
            signature: None,
            last_order_key: None,
        }
    }

    pub fn set_order_key(&mut self, order_key: OrderKey) {
        self.order_key = order_key;
    }

    pub fn last_inserted_order(&self) -> Option<OrderKey> {
        self.last_order_key
    }

    pub fn has_order_change(&self) -> bool {
        match self.last_order_key {
            Some(last) => last != self.order_key,
            None => false,
        }
    }

    pub fn set_key(&mut self, key: Option<String>) {
        self.previous_key = self.cell_key.clone();
        self.cell_key = key;
    }

    pub fn key(&self) -> Option<&str> {
        self.cell_key.as_deref()
    }

    pub fn previous_key(&self) -> Option<&str> {
        self.previous_key.as_deref()
    }

    pub fn set_signature(&mut self, signature: Option<String>) {
        self.signature = signature;
    }

    pub fn signature(&self) -> Option<&str> {
        self.signature.as_deref()
    }
}

pub(crate) trait ToolCardCell: HistoryCell + Clone + 'static {
    fn tool_card_key(&self) -> Option<&str>;
    fn set_tool_card_key(&mut self, key: Option<String>);
    fn dedupe_signature(&self) -> Option<String> {
        None
    }
}

pub(super) fn assign_tool_card_key<C: ToolCardCell>(
    slot: &mut ToolCardSlot,
    cell: &mut C,
    key: Option<String>,
) {
    let signature = cell.dedupe_signature();
    slot.set_key(key.clone());
    slot.set_signature(signature);
    cell.set_tool_card_key(key);
}

pub(super) fn ensure_tool_card<C: ToolCardCell>(
    chat: &mut ChatWidget<'_>,
    slot: &mut ToolCardSlot,
    cell: &C,
) -> usize {
    let signature = cell.dedupe_signature();
    if slot.signature() != signature.as_deref() {
        slot.set_signature(signature.clone());
    }
    if let Some(id) = slot.history_id
        && let Some(idx) = chat.cell_index_for_history_id(id)
            && cell_matches::<C>(chat, idx, slot, signature.as_deref()) {
                slot.cell_index = Some(idx);
                return idx;
            }

    if let Some(idx) = slot.cell_index
        && idx < chat.history_cells.len()
            && cell_matches::<C>(chat, idx, slot, signature.as_deref())
        {
            return idx;
        }

    if let Some(idx) = find_card_index::<C>(chat, slot, signature.as_deref()) {
        slot.cell_index = Some(idx);
        slot.history_id = chat.history_cell_ids.get(idx).and_then(|slot| *slot);
        return idx;
    }

    let idx = chat.history_insert_with_key_global(Box::new(cell.clone()), slot.order_key);
    slot.cell_index = Some(idx);
    slot.history_id = chat.history_cell_ids.get(idx).and_then(|slot| *slot);
    slot.last_order_key = Some(slot.order_key);
    idx
}

pub(super) fn replace_tool_card<C: ToolCardCell>(
    chat: &mut ChatWidget<'_>,
    slot: &mut ToolCardSlot,
    cell: &C,
) -> usize {
    let mut order_changed = slot.has_order_change();

    if order_changed && TypeId::of::<C>() == TypeId::of::<AgentRunCell>()
        && should_anchor_agent_slot(chat, slot)
            && let Some(previous) = slot.last_inserted_order() {
                slot.set_order_key(previous);
                order_changed = false;
            }

    if order_changed {
        remove_existing_card::<C>(chat, slot);
    }

    let idx = ensure_tool_card(chat, slot, cell);
    chat.history_replace_at(idx, Box::new(cell.clone()));
    slot.cell_index = Some(idx);
    slot.history_id = chat.history_cell_ids.get(idx).and_then(|slot| *slot);
    slot.last_order_key = Some(slot.order_key);
    let signature = slot.signature().map(std::string::ToString::to_string);
    prune_tool_card_duplicates::<C>(chat, slot, idx, signature.as_deref());
    idx
}

fn should_anchor_agent_slot(chat: &ChatWidget<'_>, slot: &ToolCardSlot) -> bool {
    let Some(idx) = slot.cell_index else {
        return false;
    };

    for cell in chat.history_cells.iter().skip(idx + 1) {
        if cell.as_any().downcast_ref::<AgentRunCell>().is_some() {
            continue;
        }
        if cell
            .as_any()
            .downcast_ref::<CollapsibleReasoningCell>()
            .is_some()
        {
            continue;
        }
        if matches!(cell.kind(), HistoryCellType::BackgroundEvent) {
            continue;
        }
        return false;
    }

    true
}

fn remove_existing_card<C: ToolCardCell>(chat: &mut ChatWidget<'_>, slot: &mut ToolCardSlot) {
    let signature = slot.signature().map(std::string::ToString::to_string);

    let mut target_idx = if let Some(id) = slot.history_id {
        chat.cell_index_for_history_id(id).filter(|&idx| cell_matches::<C>(chat, idx, slot, signature.as_deref()))
    } else {
        None
    };

    if target_idx.is_none()
        && let Some(idx) = slot.cell_index
            && idx < chat.history_cells.len() && cell_matches::<C>(chat, idx, slot, signature.as_deref()) {
                target_idx = Some(idx);
            }

    if target_idx.is_none() {
        target_idx = find_card_index::<C>(chat, slot, signature.as_deref());
    }

    if let Some(idx) = target_idx {
        chat.history_remove_at(idx);
    }

    slot.cell_index = None;
    slot.history_id = None;
}

fn prune_tool_card_duplicates<C: ToolCardCell>(
    chat: &mut ChatWidget<'_>,
    slot: &mut ToolCardSlot,
    keep_idx: usize,
    signature: Option<&str>,
) {
    let key = slot.key();
    let mut removals: Vec<usize> = Vec::new();
    for (idx, cell) in chat.history_cells.iter().enumerate() {
        if idx == keep_idx {
            continue;
        }
        let Some(typed) = cell.as_any().downcast_ref::<C>() else {
            continue;
        };

        let key_match = match (key, typed.tool_card_key()) {
            (Some(expected), Some(actual)) => actual == expected,
            (Some(_), None) => false,
            (None, None) => true,
            (None, Some(_)) => false,
        };

        let signature_match = match (signature, typed.dedupe_signature().as_deref()) {
            (Some(lhs), Some(rhs)) => lhs == rhs,
            _ => false,
        };

        if key_match || signature_match {
            removals.push(idx);
        }
    }

    if removals.is_empty() {
        return;
    }

    removals.sort_unstable();
    for idx in removals.into_iter().rev() {
        chat.history_remove_at(idx);
    }

    if let Some(new_idx) = find_card_index::<C>(chat, slot, signature) {
        slot.cell_index = Some(new_idx);
        slot.history_id = chat.history_cell_ids.get(new_idx).and_then(|slot| *slot);
    } else {
        slot.cell_index = None;
        slot.history_id = None;
    }
}

fn find_card_index<C: ToolCardCell>(
    chat: &ChatWidget<'_>,
    slot: &ToolCardSlot,
    signature: Option<&str>,
) -> Option<usize> {
    chat.history_cells.iter().enumerate().find_map(|(idx, cell)| {
        let typed = cell.as_any().downcast_ref::<C>()?;
        if identity_matches(typed, slot, signature) {
            Some(idx)
        } else {
            None
        }
    })
}

fn cell_matches<C: ToolCardCell>(
    chat: &ChatWidget<'_>,
    idx: usize,
    slot: &ToolCardSlot,
    signature: Option<&str>,
) -> bool {
    chat
        .history_cells
        .get(idx)
        .and_then(|cell| cell.as_any().downcast_ref::<C>())
        .map(|typed| identity_matches(typed, slot, signature))
        .unwrap_or(false)
}

fn identity_matches<C: ToolCardCell>(typed: &C, slot: &ToolCardSlot, signature: Option<&str>) -> bool {
    let mut key_match = match (slot.key(), typed.tool_card_key()) {
        (Some(expected), Some(actual)) => actual == expected,
        (Some(_), None) => false,
        (None, _) => true,
    };

    if !key_match
        && let (Some(previous), Some(actual)) = (slot.previous_key(), typed.tool_card_key()) {
            key_match = actual == previous;
        }
    let signature_match = match (signature, typed.dedupe_signature().as_deref()) {
        (Some(lhs), Some(rhs)) => lhs == rhs,
        _ => false,
    };
    key_match || signature_match
}
