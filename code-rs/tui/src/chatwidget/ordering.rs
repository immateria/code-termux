use super::*;

impl ChatWidget<'_> {
    pub(super) fn raw_order_key_from_order_meta(om: &code_core::protocol::OrderMeta) -> OrderKey {
        // sequence_number can be None on some terminal events; treat as 0 for stable placement
        OrderKey {
            req: om.request_ordinal,
            out: om.output_index.map(|v| v as i32).unwrap_or(0),
            seq: om.sequence_number.unwrap_or(0),
        }
    }

    pub(super) fn provider_order_key_from_order_meta(&mut self, om: &code_core::protocol::OrderMeta) -> OrderKey {
        let mut key = Self::raw_order_key_from_order_meta(om);
        key.req = self.apply_request_bias(key.req);
        key
    }

    pub(super) fn apply_request_bias(&mut self, provider_req: u64) -> u64 {
        if self.resume_provider_baseline.is_none()
            && let Some(target) = self.resume_expected_next_request {
                self.resume_provider_baseline = Some(provider_req);
                if provider_req <= target {
                    self.order_request_bias = target.saturating_sub(provider_req);
                } else {
                    self.order_request_bias = 0;
                }
                self.resume_expected_next_request = None;
            }
        provider_req.saturating_add(self.order_request_bias)
    }

    // Track latest request index observed from provider so internal inserts can anchor to it.
    pub(super) fn note_order(&mut self, order: Option<&code_core::protocol::OrderMeta>) {
        if let Some(om) = order {
            let is_background_sentinel = om.output_index == Some(i32::MAX as u32);
            let is_initial_session = self.last_seen_request_index == 0;
            if is_background_sentinel && is_initial_session {
                return;
            }
            let req = self.apply_request_bias(om.request_ordinal);
            self.last_seen_request_index = self.last_seen_request_index.max(req);
        }
    }

    pub(super) fn debug_fmt_order_key(ok: OrderKey) -> String {
        format!("O:req={} out={} seq={}", ok.req, ok.out, ok.seq)
    }

    pub(super) fn order_key_successor(after: OrderKey) -> OrderKey {
        if after.seq != u64::MAX {
            return OrderKey {
                req: after.req,
                out: after.out,
                seq: after.seq.saturating_add(1),
            };
        }

        if after.out != i32::MAX {
            return OrderKey {
                req: after.req,
                out: after.out.saturating_add(1),
                seq: 0,
            };
        }

        OrderKey {
            req: after.req.saturating_add(1),
            out: i32::MIN,
            seq: 0,
        }
    }

    // Allocate a key that places an internal (non‑model) event at the point it
    // occurs during the current request, instead of sinking it to the end.
    //
    // Strategy:
    // - If an OrderMeta is provided, honor it (strict model ordering).
    // - Otherwise, if a new turn is queued (a user prompt was just inserted),
    //   anchor immediately after that prompt within the upcoming request so
    //   the notice appears in the right window.
    // - Otherwise, derive a key within the current request:
    //   * If there is any existing cell in this request, append after the
    //     latest key in this request (req = last_seen, out/seq bumped).
    //   * If no cells exist for this request yet, place near the top of this
    //     request (after headers/prompts) so provider output can follow.
    pub(super) fn near_time_key(&mut self, order: Option<&code_core::protocol::OrderMeta>) -> OrderKey {
        if let Some(om) = order {
            return self.provider_order_key_from_order_meta(om);
        }

        // If we just staged a user prompt for the next request, keep using the
        // next‑turn anchor so the background item lands with that turn.
        if self.pending_user_prompts_for_next_turn > 0 {
            return self.next_req_key_after_prompt();
        }

        let req = if self.last_seen_request_index > 0 {
            self.last_seen_request_index
        } else {
            // No provider traffic yet: allocate a synthetic request bucket.
            // Use the same path as next_internal_key() to keep monotonicity.
            if self.current_request_index < self.last_seen_request_index {
                self.current_request_index = self.last_seen_request_index;
            }
            self.current_request_index = self.current_request_index.saturating_add(1);
            self.current_request_index
        };

        // Scan for the latest key within this request to append after.
        let mut last_in_req: Option<OrderKey> = None;
        for k in &self.cell_order_seq {
            if k.req == req {
                last_in_req = Some(match last_in_req {
                    Some(prev) => {
                        if *k > prev {
                            *k
                        } else {
                            prev
                        }
                    }
                    None => *k,
                });
            }
        }

        self.internal_seq = self.internal_seq.saturating_add(1);
        match last_in_req {
            Some(last) => OrderKey {
                req,
                out: last.out,
                seq: last.seq.saturating_add(1),
            },
            None => OrderKey {
                req,
                out: i32::MIN + 2,
                seq: self.internal_seq,
            },
        }
    }

    /// Like near_time_key but never advances to the next request when a prompt is queued.
    /// Use this for late, provider-origin items that lack OrderMeta (e.g., PlanUpdate)
    /// so they remain attached to the current/last request instead of jumping forward.
    pub(super) fn near_time_key_current_req(
        &mut self,
        order: Option<&code_core::protocol::OrderMeta>,
    ) -> OrderKey {
        if let Some(om) = order {
            return self.provider_order_key_from_order_meta(om);
        }
        let req = if self.last_seen_request_index > 0 {
            self.last_seen_request_index
        } else {
            if self.current_request_index < self.last_seen_request_index {
                self.current_request_index = self.last_seen_request_index;
            }
            self.current_request_index = self.current_request_index.saturating_add(1);
            self.current_request_index
        };

        let mut last_in_req: Option<OrderKey> = None;
        for k in &self.cell_order_seq {
            if k.req == req {
                last_in_req = Some(match last_in_req {
                    Some(prev) => {
                        if *k > prev {
                            *k
                        } else {
                            prev
                        }
                    }
                    None => *k,
                });
            }
        }
        self.internal_seq = self.internal_seq.saturating_add(1);
        match last_in_req {
            Some(last) => OrderKey {
                req,
                out: last.out,
                seq: last.seq.saturating_add(1),
            },
            None => OrderKey {
                req,
                out: i32::MIN + 2,
                seq: self.internal_seq,
            },
        }
    }

    // After inserting a non‑reasoning cell during streaming, restore the
    // in‑progress indicator on the latest reasoning cell so the ellipsis
    // remains visible while the model continues.
    pub(super) fn restore_reasoning_in_progress_if_streaming(&mut self) {
        if !self.stream.is_write_cycle_active() {
            return;
        }
        if let Some(idx) = self.history_cells.iter().rposition(|c| {
            c.as_any()
                .downcast_ref::<crate::history_cell::CollapsibleReasoningCell>()
                .is_some()
        })
            && let Some(rc) = self.history_cells[idx]
                .as_any()
                .downcast_ref::<crate::history_cell::CollapsibleReasoningCell>()
            {
                rc.set_in_progress(true);
            }
    }

    pub(super) fn apply_plan_terminal_title(&mut self, title: Option<String>) {
        if self.active_plan_title == title {
            return;
        }
        self.active_plan_title = title.clone();
        self.app_event_tx
            .send(AppEvent::SetTerminalTitle { title });
    }
    // Allocate a new synthetic key for internal (non-LLM) messages at the bottom of the
    // current (active) request: (req = last_seen, out = +∞, seq = monotonic).
    pub(super) fn next_internal_key(&mut self) -> OrderKey {
        // Anchor to the current provider request if known; otherwise step a synthetic counter.
        let mut req = if self.last_seen_request_index > 0 {
            self.last_seen_request_index
        } else {
            // Ensure current_request_index always moves forward
            if self.current_request_index < self.last_seen_request_index {
                self.current_request_index = self.last_seen_request_index;
            }
            self.current_request_index = self.current_request_index.saturating_add(1);
            self.current_request_index
        };
        if self.pending_user_prompts_for_next_turn > 0 {
            let next_req = self.last_seen_request_index.saturating_add(1);
            if req < next_req {
                req = next_req;
            }
        }
        if self.current_request_index < req {
            self.current_request_index = req;
        }
        self.internal_seq = self.internal_seq.saturating_add(1);
        // Place internal notices at the end of the current request window by using
        // a maximal out so they sort after any model-provided output_index.
        OrderKey {
            req,
            out: i32::MAX,
            seq: self.internal_seq,
        }
    }

    pub(super) const fn context_order_key() -> OrderKey {
        OrderKey {
            req: 0,
            out: -50,
            seq: 0,
        }
    }
}
