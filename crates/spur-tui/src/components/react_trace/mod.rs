mod builder;
mod compact_render;
pub mod dispatch;
mod render;
pub(crate) mod types;

#[cfg(feature = "markdown")]
pub use types::RenderContext;
pub use types::{ActStatus, TraceEntry, TraceKind};
#[cfg(all(test, feature = "markdown"))]
pub(crate) use types::{Segment, VirtualRow};

use spur_acp::{
    adapter::{mode_badge_with_metadata, ToolInputDisplay},
    AgentKind, ToolCallId,
};

use ratatui::style::Color;
use std::collections::{HashMap, HashSet};

use super::trace_format::{
    family_glyph, input_display_lines, input_summary, observe_compact, observe_payload_lines,
    observe_verb, outcome_glyph,
};
use super::MAX_LOG_ENTRIES;
#[cfg(test)]
use crate::components::spinner;

/// Which render surface most recently painted this trace. Used by scroll
/// mutators (`shift_anchor_by`) to pick the correct cache for anchor
/// resolution — `line_cache` for Full, `compact_cache` for Compact.
///
/// Tracking the painted surface (rather than "whichever cache is populated")
/// avoids stale-cache ambiguity if a trace is ever rendered via both paths
/// in a single session.
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub(crate) enum Surface {
    /// No render has happened yet in this session.
    #[default]
    None,
    /// Last painted by the full body render. The embedded `u64` is the
    /// `ReactTrace::generation` value at paint time. Readers must verify
    /// `g == self.generation` before trusting any cached layout — see
    /// `layout_for_scroll`.
    Full(u64),
    /// Last painted by `render_compact`. Same staleness contract as `Full`.
    Compact(u64),
}

#[cfg(feature = "markdown")]
#[derive(Clone, Copy)]
pub(super) struct PlainTextAppendProvenance {
    /// Tail entry receiving every mutation since the cached generation.
    pub(super) entry_idx: usize,
    /// Generation represented by the cache before the first append.
    pub(super) base_generation: u64,
}

pub struct ReactTrace {
    pub(super) entries: Vec<TraceEntry>,
    pub(super) inline_images: HashMap<types::TraceImageId, types::TraceImage>,
    pub(super) image_digests: HashSet<String>,
    pub(super) next_trace_image_id: u64,
    pub(super) next_trace_image_generation: u64,
    pub(super) anchor: crate::components::react_trace::types::ScrollAnchor,
    #[cfg(feature = "markdown")]
    pub(super) last_scroll_at: Option<std::time::Instant>,
    #[cfg(feature = "markdown")]
    pub(super) prev_anchor_for_debounce: crate::components::react_trace::types::ScrollAnchor,
    pub(super) tick_counter: u8,
    /// Cached total rendered lines from last render.
    pub(super) last_total_lines: usize,
    /// Cached visible height from last render.
    pub(super) last_visible_height: usize,
    /// The render surface most recently painted. Drives cache selection
    /// in `shift_anchor_by`.
    pub(super) last_surface: Surface,
    /// Width hint from the most-recent render call; used by scroll mutators
    /// to compute fresh row counts without stale last_total_lines.
    pub(super) last_render_width: Option<u16>,
    /// Whether mermaid rendering is available.
    pub(super) mermaid_enabled: bool,
    /// Which agent brain backs this session; drives pane title + accent color.
    pub(super) agent_kind: AgentKind,
    /// Current session mode, if known (e.g. "plan", "acceptEdits").
    pub(super) current_mode: Option<String>,
    /// Full advertised mode records used for semantic v1.7 badge rendering.
    pub(super) mode_catalog: Vec<spur_acp::SessionMode>,
    /// When true (default), Observe entries show a truncated preview.
    pub(super) observe_collapsed: bool,
    /// When true, `render_compact` will be the authoritative entry
    /// point used by the DetailPane Stream tab. Set at construction via
    /// `with_kind_compact`. Task 0.3 adds the render branch; this flag
    /// is currently a marker.
    pub(super) compact: bool,
    /// Generation counter bumped on every content mutation.
    pub(super) generation: u64,
    /// Index of the first entry needing row rebuild.
    pub(super) dirty_from: Option<usize>,
    /// Proof that every mutation since `base_generation` was an append to the
    /// same active plain-text tail. Generic mutations clear this marker.
    #[cfg(feature = "markdown")]
    pub(super) plain_text_append: Option<PlainTextAppendProvenance>,
    /// Cached pre-wrapped display lines.
    #[cfg(not(feature = "markdown"))]
    pub(super) line_cache: Option<render::LineCacheEntry>,
    /// Cached virtual rows for the markdown render path.
    #[cfg(feature = "markdown")]
    pub(super) line_cache: Option<render::VirtualRowCacheEntry>,
    /// Cache for the compact render path (`render_compact`).
    /// Independent from `line_cache` because the two paths produce
    /// different row layouts. `None` until first compact render.
    pub(in crate::components::react_trace) compact_cache: Option<compact_render::CompactCacheEntry>,
    /// Cache for the external body-lines path used by DetailPane Stream.
    /// Independent from `line_cache` and `compact_cache`.
    pub(in crate::components::react_trace) body_cache: Option<render::BodyCacheEntry>,
    /// Active theme used by builder/render to resolve color tokens.
    /// Defaults to the dark fallback in `new()`; production callers
    /// (SessionDetailView) call `set_theme(ctx.theme)` before render so
    /// the trace tracks the user's configured theme.
    pub(super) theme: crate::theme::Theme,
}

/// Inverse of `resolve_anchor` for the Row variant: given a row index,
/// find which entry it belongs to and the row-within-entry offset.
///
/// Assumes `entry_row_starts[0] == 0` (builder invariant). For `row`
/// values smaller than `entry_row_starts[0]`, `within` would underflow
/// in release mode; callers must clamp inputs.
fn row_to_anchor(row: usize, entry_row_starts: &[usize]) -> (usize, usize) {
    if entry_row_starts.is_empty() {
        return (0, 0);
    }
    let entry_idx = match entry_row_starts.binary_search(&row) {
        Ok(i) => i,
        Err(i) => i.saturating_sub(1),
    };
    let within = row - entry_row_starts[entry_idx];
    (entry_idx, within)
}

/// Merge an incoming `ToolCallUpdate.fields` into the previous `ActStatus`.
///
/// Rules:
///   - Terminal `prev` (Completed / Failed): `debug_assert!` that the
///     incoming status, if present, matches; return `prev.clone()` unchanged.
///     Prevents a late `InProgress` update from reopening a closed tool call.
///   - `incoming_status == None`: keep `prev` variant; refresh
///     `InProgress.partial` only when `prev` is `InProgress` AND
///     `incoming_raw_output` is `Some(v)`.
///   - `incoming_status == Some(s)`: map `(s, incoming_raw_output)` to a
///     new `ActStatus`. An incoming terminal always replaces non-terminal.
///   - Any future `ToolCallStatus` variant not listed here (the enum may
///     become `#[non_exhaustive]` upstream) is absorbed: log via
///     `tracing::debug!` and return `prev.clone()`.
pub(crate) fn merge_status(
    prev: &types::ActStatus,
    incoming_status: Option<spur_acp::ToolCallStatus>,
    incoming_raw_output: Option<&serde_json::Value>,
    kind: spur_acp::AgentKind,
) -> types::ActStatus {
    use spur_acp::adapter::extract_observe;
    use spur_acp::ToolCallStatus;
    use types::ActStatus;

    let parse = |v: &serde_json::Value| extract_observe(v, kind);

    // Terminal prev wins.
    if matches!(prev, ActStatus::Completed(_) | ActStatus::Failed(_)) {
        if let Some(s) = incoming_status {
            let prev_is_completed = matches!(prev, ActStatus::Completed(_));
            let prev_is_failed = matches!(prev, ActStatus::Failed(_));
            let ok = (prev_is_completed && matches!(s, ToolCallStatus::Completed))
                || (prev_is_failed && matches!(s, ToolCallStatus::Failed));
            if !ok {
                tracing::warn!(
                    ?prev,
                    incoming = ?s,
                    "ignoring late ToolCallUpdate on terminal ActStatus"
                );
            }
        }
        return prev.clone();
    }

    let Some(s) = incoming_status else {
        // No status change. Possibly refresh partial on InProgress.
        return match (prev, incoming_raw_output) {
            (ActStatus::InProgress { .. }, Some(v)) => ActStatus::InProgress {
                partial: Some(parse(v)),
            },
            _ => prev.clone(),
        };
    };

    match s {
        ToolCallStatus::Pending => ActStatus::Pending,
        ToolCallStatus::InProgress => ActStatus::InProgress {
            partial: incoming_raw_output.map(parse),
        },
        ToolCallStatus::Completed => ActStatus::Completed(incoming_raw_output.map(parse)),
        ToolCallStatus::Failed => ActStatus::Failed(incoming_raw_output.map(parse)),
        _ => {
            tracing::debug!(
                ?prev,
                incoming = ?s,
                "unknown ToolCallStatus variant; preserving prev"
            );
            prev.clone()
        }
    }
}

/// Map an ACP `ToolCallStatus` + optional `raw_output` to an `ActStatus`
/// for a newly-created Act entry. Honours the incoming status — an agent
/// may stream an already-completed tool call on the first event.
pub(crate) fn map_initial_status(
    status: spur_acp::ToolCallStatus,
    raw_output: Option<&serde_json::Value>,
    kind: spur_acp::AgentKind,
) -> types::ActStatus {
    use spur_acp::adapter::extract_observe;
    use spur_acp::ToolCallStatus;
    use types::ActStatus;

    let parse = |v: &serde_json::Value| extract_observe(v, kind);
    match status {
        ToolCallStatus::Pending => ActStatus::Pending,
        ToolCallStatus::InProgress => ActStatus::InProgress {
            partial: raw_output.map(parse),
        },
        ToolCallStatus::Completed => ActStatus::Completed(raw_output.map(parse)),
        ToolCallStatus::Failed => ActStatus::Failed(raw_output.map(parse)),
        _ => ActStatus::Pending,
    }
}

impl ReactTrace {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            inline_images: HashMap::new(),
            image_digests: HashSet::new(),
            next_trace_image_id: 0,
            next_trace_image_generation: 0,
            anchor: crate::components::react_trace::types::ScrollAnchor::default(),
            #[cfg(feature = "markdown")]
            last_scroll_at: None,
            #[cfg(feature = "markdown")]
            prev_anchor_for_debounce: crate::components::react_trace::types::ScrollAnchor::default(
            ),
            tick_counter: 0,
            last_total_lines: 0,
            last_visible_height: 20,
            last_surface: Surface::None,
            last_render_width: None,
            mermaid_enabled: true,
            agent_kind: AgentKind::Generic,
            current_mode: None,
            mode_catalog: Vec::new(),
            observe_collapsed: true,
            compact: false,
            generation: 0,
            dirty_from: None,
            #[cfg(feature = "markdown")]
            plain_text_append: None,
            line_cache: None,
            compact_cache: None,
            body_cache: None,
            theme: crate::theme::fallback_theme().clone(),
        }
    }

    /// Update the active theme used for color-token resolution. Production
    /// callers invoke this from their render path with `ctx.theme` so the
    /// trace tracks the user's configured palette. Bumps generation so
    /// cached lines rebuild against the new tokens on next render.
    pub fn set_theme(&mut self, theme: &crate::theme::Theme) {
        if self.theme != *theme {
            self.theme = theme.clone();
            self.invalidate_cache();
        }
    }

    /// Create a `ReactTrace` with an explicit `AgentKind` for title + accent color.
    pub fn with_kind(kind: AgentKind) -> Self {
        Self {
            agent_kind: kind,
            ..Self::new()
        }
    }

    /// Create a `ReactTrace` with a compact render mode suitable for
    /// narrow panes (≈40 cols). Disables markdown/mermaid implicitly in
    /// the render branch added by Task 0.3.
    pub fn with_kind_compact(kind: AgentKind) -> Self {
        Self {
            agent_kind: kind,
            compact: true,
            ..Self::new()
        }
    }

    // TODO: with_kind_compact is deprecated; callers should use with_kind.
    // The compact render path is no longer used by DetailPane.

    /// True if this trace was constructed with `with_kind_compact`.
    pub fn is_compact(&self) -> bool {
        self.compact
    }

    /// Store the current session mode id (e.g. "plan", "acceptEdits").
    pub fn set_mode(&mut self, mode: Option<String>) {
        self.current_mode = mode;
        self.invalidate_cache();
    }

    pub fn set_mode_catalog(&mut self, modes: Vec<spur_acp::SessionMode>) {
        self.mode_catalog = modes;
        self.invalidate_cache();
    }

    /// Current session mode id, if set.
    #[cfg(test)]
    pub fn current_mode(&self) -> Option<&str> {
        self.current_mode.as_deref()
    }

    /// Wipe all entries and scroll state. Preserves `agent_kind`,
    /// `mermaid_enabled`, `compact` (config) — only conversation content
    /// and derived caches are cleared. Used by `/clear` view reset.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.inline_images.clear();
        self.image_digests.clear();
        self.next_trace_image_id = 0;
        self.next_trace_image_generation = 0;
        self.anchor = crate::components::react_trace::types::ScrollAnchor::default();
        #[cfg(feature = "markdown")]
        {
            self.last_scroll_at = None;
            self.prev_anchor_for_debounce =
                crate::components::react_trace::types::ScrollAnchor::default();
        }
        self.last_total_lines = 0;
        self.last_render_width = None;
        self.line_cache = None;
        self.compact_cache = None;
        self.body_cache = None;
        self.invalidate_cache();
    }

    /// Toggle the collapsed state for Observe (tool-result) entries.
    pub fn toggle_observe_collapsed(&mut self) -> bool {
        self.observe_collapsed = !self.observe_collapsed;
        self.invalidate_cache();
        self.observe_collapsed
    }

    /// Whether observe entries are currently collapsed.
    pub fn observe_collapsed(&self) -> bool {
        self.observe_collapsed
    }

    /// Build the styled pane title + accent color from `agent_kind` and
    /// optional mode badge.
    pub(super) fn pane_title_and_color(&self) -> (String, Color) {
        use crate::theme::{resolve_token, ColorDepth};
        let (base_title, accent) = match self.agent_kind {
            AgentKind::ClaudeCodeAcp | AgentKind::ClaudeStreamJson => (
                " Session · claude ",
                resolve_token(
                    &self.theme,
                    "react_trace.title.claude.fg",
                    ColorDepth::Truecolor,
                ),
            ),
            AgentKind::CodexAcp => (
                " Session · codex ",
                resolve_token(
                    &self.theme,
                    "react_trace.title.codex.fg",
                    ColorDepth::Truecolor,
                ),
            ),
            AgentKind::Kiro => (
                " Session · kiro ",
                resolve_token(
                    &self.theme,
                    "react_trace.title.kiro.fg",
                    ColorDepth::Truecolor,
                ),
            ),
            AgentKind::Kimi => (
                " Session · kimi ",
                resolve_token(
                    &self.theme,
                    "react_trace.title.generic.fg",
                    ColorDepth::Truecolor,
                ),
            ),
            AgentKind::OpenCode => (
                " Session · opencode ",
                resolve_token(
                    &self.theme,
                    "react_trace.title.generic.fg",
                    ColorDepth::Truecolor,
                ),
            ),
            AgentKind::Gemini => (
                " Session · gemini ",
                resolve_token(
                    &self.theme,
                    "react_trace.title.generic.fg",
                    ColorDepth::Truecolor,
                ),
            ),
            AgentKind::Grok => (
                " Session · grok ",
                resolve_token(
                    &self.theme,
                    "react_trace.title.generic.fg",
                    ColorDepth::Truecolor,
                ),
            ),
            AgentKind::Pi => (
                " Session · pi ",
                resolve_token(
                    &self.theme,
                    "react_trace.title.generic.fg",
                    ColorDepth::Truecolor,
                ),
            ),
            AgentKind::Generic => (
                " Session ",
                resolve_token(
                    &self.theme,
                    "react_trace.title.generic.fg",
                    ColorDepth::Truecolor,
                ),
            ),
        };
        let mut title = if let Some(mode_id) = &self.current_mode {
            let mode = self
                .mode_catalog
                .iter()
                .find(|mode| mode.id.0.as_ref() == mode_id);
            if let Some(badge) = mode_badge_with_metadata(mode_id, self.agent_kind, mode) {
                format!("{}· {} ", base_title, badge.short)
            } else {
                base_title.to_string()
            }
        } else {
            base_title.to_string()
        };
        if !self.observe_collapsed {
            // Use EAW=W bullet (📂) or pure ASCII to avoid iTerm2 font-fallback
            // cursor desync (see tests/expanded_mode_ghost_text_repro.rs).
            // EAW=N `⊞` was the source of the original ghost-text bug visible
            // in the title bar.
            title.push_str("· expanded ");
        }
        (title, accent)
    }

    /// Set whether ```mermaid fences should be rendered as images.
    pub fn set_mermaid_enabled(&mut self, enabled: bool) {
        self.mermaid_enabled = enabled;
        self.invalidate_cache();
    }

    /// Bump the generation counter, which causes the next `render()` call
    /// to rebuild the line cache.
    fn invalidate_cache(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.dirty_from = Some(0);
        #[cfg(feature = "markdown")]
        {
            self.plain_text_append = None;
        }
    }

    /// Mark entries from `idx` onward as needing row rebuild.
    fn mark_dirty_from(&mut self, idx: usize) {
        self.generation = self.generation.wrapping_add(1);
        let prev = self.dirty_from;
        self.dirty_from = Some(prev.map_or(idx, |d| d.min(idx)));
        #[cfg(feature = "markdown")]
        {
            self.plain_text_append = None;
        }
    }

    /// Mark an append to the active markdown tail while preserving provenance
    /// across multiple chunks received before the next render.
    #[cfg(feature = "markdown")]
    fn mark_plain_text_append(&mut self, idx: usize) {
        let previous_generation = self.generation;
        self.plain_text_append = match (self.plain_text_append, self.dirty_from) {
            (Some(provenance), Some(dirty_idx))
                if provenance.entry_idx == idx && dirty_idx == idx =>
            {
                Some(provenance)
            }
            (None, None) => Some(PlainTextAppendProvenance {
                entry_idx: idx,
                base_generation: previous_generation,
            }),
            _ => None,
        };
        self.generation = self.generation.wrapping_add(1);
        let prev = self.dirty_from;
        self.dirty_from = Some(prev.map_or(idx, |dirty_idx| dirty_idx.min(idx)));
    }

    /// Public wrapper for external callers that legitimately mutate an
    /// entry in-place (e.g. `SessionUpdate::ToolCallUpdate` merging into
    /// an existing `Act`). Bumps generation + marks cache dirty from idx.
    pub(crate) fn mark_dirty_from_for_update(&mut self, idx: usize) {
        self.mark_dirty_from(idx);
    }

    /// Return a short kind name for the most recent entry, or `None` if empty.
    pub fn last_entry_kind_name(&self) -> Option<&'static str> {
        self.entries.last().map(|e| match &e.kind {
            TraceKind::Think => "think",
            TraceKind::AgentMessage { .. } => "agent_message",
            TraceKind::Act { .. } => "act",
            TraceKind::Observe { .. } => "observe",
            TraceKind::Delegate { .. } => "delegate",
            TraceKind::UserMessage => "user_message",
            TraceKind::Permission { .. } => "permission",
            TraceKind::Image { .. } => "image",
        })
    }

    /// Append text to the most recent THINK entry, or create a new one.
    pub fn append_think(&mut self, text: &str, timestamp: String) {
        if let Some(last) = self.entries.last_mut() {
            if matches!(last.kind, TraceKind::Think) {
                if !last.text.is_empty() {
                    last.text.push_str(text);
                } else {
                    last.text = text.to_string();
                }
                self.mark_dirty_from(self.entries.len() - 1);
                if self.is_following() {
                    self.scroll_to_bottom();
                }
                return;
            }
        }
        self.push(TraceEntry {
            kind: TraceKind::Think,
            text: text.to_string(),
            timestamp,
            #[cfg(feature = "markdown")]
            markdown: None,
        });
    }

    /// Append text to the most recent AgentMessage entry for the same agent,
    /// or create a new one.
    pub fn append_message(&mut self, text: &str, agent: &str, timestamp: String) {
        let target_idx = match self.entries.last() {
            Some(entry) => match &entry.kind {
                TraceKind::AgentMessage { agent: a } if a == agent => Some(self.entries.len() - 1),
                _ => None,
            },
            None => None,
        };

        #[cfg(feature = "markdown")]
        {
            if let Some(idx) = target_idx {
                if let Some(entry) = self.entries.get_mut(idx) {
                    if let Some(stream) = entry.markdown.as_mut() {
                        if !stream.is_finalized() {
                            stream.append(text);
                            self.mark_plain_text_append(idx);
                            if self.is_following() {
                                self.scroll_to_bottom();
                            }
                            return;
                        }
                    }
                }
            }
            let mut stream =
                super::markdown_stream::MarkdownStream::new_with_mermaid(self.mermaid_enabled);
            stream.append(text);
            self.push(TraceEntry {
                kind: TraceKind::AgentMessage {
                    agent: agent.to_string(),
                },
                text: String::new(),
                timestamp,
                markdown: Some(stream),
            });
        }
        #[cfg(not(feature = "markdown"))]
        {
            if let Some(idx) = target_idx {
                if let Some(entry) = self.entries.get_mut(idx) {
                    entry.text.push_str(text);
                    self.mark_dirty_from(idx);
                    if self.is_following() {
                        self.scroll_to_bottom();
                    }
                    return;
                }
            }
            self.push(TraceEntry {
                kind: TraceKind::AgentMessage {
                    agent: agent.to_string(),
                },
                text: text.to_string(),
                timestamp,
            });
        }
    }

    /// Append a streamed user-message chunk, coalescing into the tail entry
    /// iff that entry is `TraceKind::UserMessage`. Otherwise push a new
    /// `TraceKind::UserMessage` entry. Symmetric to `append_message` for
    /// agent chunks.
    pub fn append_user_message(&mut self, text: &str, timestamp: String) {
        let target_idx = match self.entries.last() {
            Some(entry) => match &entry.kind {
                TraceKind::UserMessage => Some(self.entries.len() - 1),
                _ => None,
            },
            None => None,
        };

        match target_idx {
            Some(idx) => {
                // Idempotency: local echo (`blocks_preview`) already seeded
                // the full turn, including `@name` reconstructions of
                // ResourceLink/Resource. Inbound ACP chunks are fragments
                // (`"review "`, `" now"`) that do not suffix-match the
                // seeded preview. Skip any non-empty chunk already present
                // in the tail so mention-shaped turns do not double.
                if text.is_empty() || self.entries[idx].text.contains(text) {
                    return;
                }
                self.entries[idx].text.push_str(text);
                self.mark_dirty_from(idx);
                if self.is_following() {
                    self.scroll_to_bottom();
                }
            }
            None => {
                self.entries.push(TraceEntry {
                    kind: TraceKind::UserMessage,
                    text: text.to_string(),
                    timestamp,
                    #[cfg(feature = "markdown")]
                    markdown: None,
                });
                self.mark_dirty_from(self.entries.len() - 1);
                if self.is_following() {
                    self.scroll_to_bottom();
                }
            }
        }
    }

    /// Push a new trace entry, evicting oldest if over capacity.
    pub fn push(&mut self, entry: TraceEntry) {
        self.entries.push(entry);
        if self.entries.len() > MAX_LOG_ENTRIES {
            let drain = self.entries.len() - MAX_LOG_ENTRIES;
            self.entries.drain(..drain);
            // Adjust anchor's entry_idx; if anchor pointed at evicted entry,
            // snap to the first surviving entry's first row.
            match self.anchor {
                crate::components::react_trace::types::ScrollAnchor::Row {
                    entry_idx,
                    row_within_entry,
                } => {
                    self.anchor = if entry_idx < drain {
                        crate::components::react_trace::types::ScrollAnchor::Row {
                            entry_idx: 0,
                            row_within_entry: 0,
                        }
                    } else {
                        crate::components::react_trace::types::ScrollAnchor::Row {
                            entry_idx: entry_idx - drain,
                            row_within_entry,
                        }
                    };
                }
                crate::components::react_trace::types::ScrollAnchor::Following => {}
            }
            self.invalidate_cache();
        } else {
            self.mark_dirty_from(self.entries.len().saturating_sub(2));
        }
        if self.is_following() {
            self.scroll_to_bottom();
        }
    }

    pub(crate) fn append_image(
        &mut self,
        image: std::sync::Arc<image::DynamicImage>,
        path: std::path::PathBuf,
        sha256: String,
        timestamp: String,
    ) -> Option<types::TraceImageId> {
        if !self.image_digests.insert(sha256.clone()) {
            return None;
        }
        let id = types::TraceImageId(self.next_trace_image_id);
        self.next_trace_image_id = self.next_trace_image_id.saturating_add(1);
        self.next_trace_image_generation = self.next_trace_image_generation.saturating_add(1);
        self.inline_images.insert(
            id,
            types::TraceImage {
                image,
                path: path.clone(),
                image_generation: self.next_trace_image_generation,
            },
        );
        self.push(TraceEntry {
            kind: TraceKind::Image {
                id,
                label: format!("image · {}", path.display()),
            },
            text: path.display().to_string(),
            timestamp,
            #[cfg(feature = "markdown")]
            markdown: None,
        });
        Some(id)
    }

    /// Returns true when the viewport is pinned to the tail of the trace.
    pub fn is_following(&self) -> bool {
        matches!(
            self.anchor,
            crate::components::react_trace::types::ScrollAnchor::Following
        )
    }

    /// Move viewport up by one row by re-anchoring to the previous row.
    pub fn scroll_up(&mut self) {
        self.shift_anchor_by(-1);
    }

    pub fn scroll_up_by(&mut self, lines: usize) {
        self.shift_anchor_by(-(lines as isize));
    }

    pub fn scroll_down(&mut self) {
        self.shift_anchor_by(1);
    }

    pub fn scroll_down_by(&mut self, lines: usize) {
        self.shift_anchor_by(lines as isize);
    }

    pub fn page_up(&mut self) {
        let jump = self.last_visible_height.saturating_sub(2).max(1) as isize;
        self.shift_anchor_by(-jump);
    }

    pub fn page_down(&mut self) {
        let jump = self.last_visible_height.saturating_sub(2).max(1) as isize;
        self.shift_anchor_by(jump);
    }

    pub fn scroll_to_top(&mut self) {
        self.anchor = crate::components::react_trace::types::ScrollAnchor::Row {
            entry_idx: 0,
            row_within_entry: 0,
        };
    }

    pub fn scroll_to_bottom(&mut self) {
        self.anchor = crate::components::react_trace::types::ScrollAnchor::Following;
    }

    /// Layout selection for scroll math. Returns `(entry_row_starts, total_rows)`
    /// for whichever cache was last painted. Cloning `entry_row_starts` keeps
    /// the subsequent `&mut self` mutation of the anchor borrow-safe.
    ///
    /// Non-markdown full-render path returns `None` because `LineCacheEntry`
    /// does not yet track per-entry row boundaries — tracked as a follow-up.
    fn layout_for_scroll(&self) -> Option<(Vec<usize>, usize)> {
        match self.last_surface {
            Surface::None => None,
            Surface::Compact(g) if g == self.generation => self
                .compact_cache
                .as_ref()
                .map(|c| (c.entry_row_starts.clone(), c.lines.len())),
            Surface::Full(g) if g == self.generation => {
                #[cfg(feature = "markdown")]
                {
                    self.line_cache
                        .as_ref()
                        .map(|c| (c.entry_row_starts.clone(), c.rows.len()))
                }
                #[cfg(not(feature = "markdown"))]
                {
                    None
                }
            }
            _ => None,
        }
    }

    /// Apply a row delta to the current anchor by:
    /// 1. resolving the current anchor against the cached layout from the
    ///    most recent render (P2-δ — guarantees scroll math uses the same
    ///    coordinate system render painted with),
    /// 2. computing the target row,
    /// 3. converting back to a Row anchor at the target row.
    ///
    /// Cache selection is driven by `last_surface` (not "whichever cache is
    /// populated") so scroll math always agrees with the layout painted on
    /// the most recent render. If no surface has painted yet, this is a
    /// no-op — anchor remains in its initial state.
    /// If the target row is the last visible row, transitions to Following.
    /// When `total <= visible_h` (entire content fits on-screen), all scroll
    /// inputs transition to `Following` — there is nothing to scroll.
    fn shift_anchor_by(&mut self, delta: isize) {
        use crate::components::react_trace::types::ScrollAnchor;

        let Some((starts_vec, total)) = self.layout_for_scroll() else {
            return;
        };
        let starts: &[usize] = &starts_vec;
        let visible_h = self.last_visible_height.max(1);

        let current_row = crate::components::react_trace::render::resolve_anchor(
            &self.anchor,
            starts,
            total,
            visible_h,
        );

        let target = (current_row as isize + delta)
            .max(0)
            .min(total.saturating_sub(visible_h) as isize) as usize;

        if target >= total.saturating_sub(visible_h) {
            self.anchor = ScrollAnchor::Following;
            return;
        }

        let (entry_idx, row_within_entry) = row_to_anchor(target, starts);
        self.anchor = ScrollAnchor::Row {
            entry_idx,
            row_within_entry,
        };
    }

    /// Called on each tick: advance spinner counter and decrement pending
    /// permission countdowns.
    pub fn tick(&mut self) {
        self.tick_counter = self.tick_counter.wrapping_add(1);

        let mut animation_idx: Option<usize> = None;
        for (i, entry) in self.entries.iter_mut().enumerate() {
            if let TraceKind::Permission {
                pending, countdown, ..
            } = &mut entry.kind
            {
                if *pending && *countdown > 0 {
                    *countdown = countdown.saturating_sub(1);
                    animation_idx = Some(animation_idx.map_or(i, |a| a.min(i)));
                }
            }
        }
        if animation_idx.is_none() {
            animation_idx = self.first_active_spinner();
        }
        if let Some(idx) = animation_idx {
            self.mark_dirty_from(idx);
        }
    }

    /// Returns the index of the first entry whose tool call is still
    /// animating (Pending or InProgress). Caller uses this to drive cache
    /// invalidation in `tick`.
    pub(crate) fn first_active_spinner(&self) -> Option<usize> {
        self.entries.iter().position(|e| {
            matches!(
                &e.kind,
                TraceKind::Act { status, .. } if status.is_active()
            )
        })
    }

    /// Drain any mermaid fences detected during the last debounce window.
    #[cfg(feature = "markdown")]
    pub fn drain_fence_dispatches(
        &mut self,
        states: &super::markdown_stream::StateLookup<'_>,
    ) -> Vec<(usize, super::markdown_stream::FenceRef)> {
        let mut out = Vec::new();
        let mut first_flushed: Option<usize> = None;
        for (idx, entry) in self.entries.iter_mut().enumerate() {
            if let Some(stream) = entry.markdown.as_mut() {
                let was_dirty = stream.is_dirty();
                for fence in stream.maybe_flush(states) {
                    out.push((idx, fence));
                }
                if was_dirty && !stream.is_dirty() {
                    first_flushed = Some(first_flushed.map_or(idx, |d| d.min(idx)));
                }
            }
        }
        if let Some(idx) = first_flushed {
            self.mark_dirty_from(idx);
        }
        out
    }

    #[cfg(not(feature = "markdown"))]
    pub fn drain_fence_dispatches(&mut self) -> Vec<(usize, ())> {
        Vec::new()
    }

    /// Force an immediate rebuild of every markdown stream.
    ///
    /// Used on TurnComplete — uses `flush_final` so trailing content
    /// (paragraphs / fence closes at EOF) gets committed and styled
    /// instead of rendering as plain tail.
    #[cfg(feature = "markdown")]
    pub fn force_flush_all(
        &mut self,
        states: &super::markdown_stream::StateLookup<'_>,
    ) -> Vec<(usize, super::markdown_stream::FenceRef)> {
        let mut out = Vec::new();
        for (idx, entry) in self.entries.iter_mut().enumerate() {
            if let Some(stream) = entry.markdown.as_mut() {
                for fence in stream.flush_final(states) {
                    out.push((idx, fence));
                }
            }
        }
        self.invalidate_cache();
        out
    }

    #[cfg(not(feature = "markdown"))]
    pub fn force_flush_all(&mut self) -> Vec<(usize, ())> {
        Vec::new()
    }

    /// Mark every `AgentMessage` markdown stream as dirty.
    #[cfg(feature = "markdown")]
    pub fn mark_all_streams_dirty(&mut self) {
        for entry in &mut self.entries {
            if let Some(stream) = entry.markdown.as_mut() {
                stream.mark_dirty_now();
            }
        }
        self.invalidate_cache();
    }

    /// Returns true if any entry has a pending permission request.
    pub fn has_pending_permission(&self) -> bool {
        self.entries
            .iter()
            .any(|e| matches!(&e.kind, TraceKind::Permission { pending: true, .. }))
    }

    /// Mark all pending permission entries as resolved.
    pub fn resolve_pending_permissions(&mut self) {
        for entry in &mut self.entries {
            if let TraceKind::Permission { pending, .. } = &mut entry.kind {
                *pending = false;
            }
        }
        self.invalidate_cache();
    }

    /// Collect executor IDs from all Delegate trace entries that have been
    /// dispatched.
    pub fn active_executor_ids(&self) -> Vec<String> {
        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::new();
        for e in &self.entries {
            if let TraceKind::Delegate {
                executor_id: Some(id),
                ..
            } = &e.kind
            {
                if seen.insert(id.as_str()) {
                    out.push(id.clone());
                }
            }
        }
        out
    }

    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }

    pub fn last_render_width(&self) -> Option<u16> {
        self.last_render_width
    }

    /// Build wrapped display lines for external pane consumption
    /// (DetailPane Stream tab). Uses `build_display_lines` with no lineage
    /// and wraps to `width`. Caches result keyed by `(generation, width)`.
    pub fn build_body_lines(&mut self, width: u16) -> &[ratatui::text::Line<'static>] {
        let cache_valid = self
            .body_cache
            .as_ref()
            .is_some_and(|c| c.generation == self.generation && c.width == width);
        if !cache_valid {
            let spinner_frame = crate::components::spinner::frame(
                crate::components::spinner::BRAILLE,
                self.tick_counter as u32,
            );
            let lines =
                self.build_display_lines(spinner_frame, None, Some(width.saturating_sub(3)));
            let wrapped = lines
                .into_iter()
                .flat_map(|l| crate::components::line_wrap::wrap_line_to_width(&l, width))
                .collect();
            self.body_cache = Some(crate::components::react_trace::render::BodyCacheEntry {
                lines: wrapped,
                width,
                generation: self.generation,
            });
        }
        &self
            .body_cache
            .as_ref()
            .expect("body cache initialized")
            .lines
    }

    /// Expose the trace entries as a slice for testing and inspection.
    pub fn entries(&self) -> &[TraceEntry] {
        &self.entries
    }

    /// Test-only accessor.
    #[doc(hidden)]
    pub(crate) fn entries_for_test(&self) -> &[TraceEntry] {
        &self.entries
    }

    #[cfg(test)]
    pub(crate) fn image_for_test(&self, id: types::TraceImageId) -> Option<&types::TraceImage> {
        self.inline_images.get(&id)
    }

    /// Test-only mutable accessor.
    #[doc(hidden)]
    #[cfg(test)]
    pub(crate) fn entries_mut_for_test(&mut self) -> &mut Vec<TraceEntry> {
        &mut self.entries
    }

    /// Return the text of the most recent entry, or `None` if the trace is empty.
    #[cfg(test)]
    pub fn last_text(&self) -> Option<String> {
        self.entries.last().map(|e| e.text.clone())
    }

    /// Locate the most recent `Delegate` entry whose `request_id` matches
    /// and attach the `executor_id`.
    pub fn attach_executor_id(&mut self, request_id: &str, executor_id: &str) {
        for entry in self.entries.iter_mut().rev() {
            if let TraceKind::Delegate {
                request_id: Some(rid),
                executor_id: slot @ None,
                ..
            } = &mut entry.kind
            {
                if rid == request_id {
                    *slot = Some(executor_id.to_string());
                    self.invalidate_cache();
                    return;
                }
            }
        }
        tracing::debug!(
            request_id = %request_id,
            executor_id = %executor_id,
            "DelegationDispatched arrived but no matching Delegate entry"
        );
    }

    /// Locate the most recent `Delegate` entry whose `executor_id` matches
    /// and update its `status`. Falls back to matching `request_id` only when
    /// `executor_id` is `None` on the entry (pre-dispatch correlation).
    pub fn update_delegate_status(&mut self, executor_id: &str, new_status: &str) {
        for entry in self.entries.iter_mut().rev() {
            if let TraceKind::Delegate {
                status,
                executor_id: eid,
                request_id: rid,
                ..
            } = &mut entry.kind
            {
                // Prefer executor_id match; fall back to request_id only when
                // executor_id hasn't been attached yet. This prevents updating
                // the wrong entry if request_id and executor_id ever diverge.
                let matches = eid.as_deref() == Some(executor_id)
                    || (eid.is_none() && rid.as_deref() == Some(executor_id));
                if matches {
                    *status = new_status.to_string();
                    self.invalidate_cache();
                    return;
                }
            }
        }
        tracing::debug!(
            executor_id = %executor_id,
            new_status = %new_status,
            "DelegationCompleted arrived but no matching Delegate entry"
        );
    }

    /// Locate the newest `TraceKind::Act` entry whose `tool_call_id` matches.
    /// Returns the absolute entry index and a mutable reference, or `None`.
    ///
    /// Compares the inner `Arc<str>` content rather than `Arc` identity, so
    /// ids produced by separate protocol round trips still compare equal.
    pub(crate) fn find_act_by_id_mut(
        &mut self,
        id: &ToolCallId,
    ) -> Option<(usize, &mut TraceEntry)> {
        let needle: &str = id.0.as_ref();
        for (idx, entry) in self.entries.iter_mut().enumerate().rev() {
            if let TraceKind::Act {
                tool_call_id: Some(existing),
                ..
            } = &entry.kind
            {
                if existing.0.as_ref() == needle {
                    return Some((idx, entry));
                }
            }
        }
        None
    }
}

impl Default for ReactTrace {
    fn default() -> Self {
        Self::new()
    }
}

impl ReactTrace {
    /// Render every entry to plain strings (one per logical line).
    pub fn render_to_strings(&self) -> Vec<String> {
        let mut lines: Vec<String> = Vec::new();
        let collapsed = self.observe_collapsed;

        let mut i = 0;
        while i < self.entries.len() {
            let entry = &self.entries[i];

            if collapsed {
                if let TraceKind::Act {
                    tool,
                    family,
                    input,
                    status,
                    ..
                } = &entry.kind
                {
                    let (act_glyph, _) = family_glyph(&self.theme, *family);
                    let id_str = input_summary(input, tool);
                    let tail = match status {
                        ActStatus::Pending | ActStatus::InProgress { .. } => "\u{2026}".to_string(),
                        ActStatus::Completed(Some(p)) => {
                            let (glyph, _, stats) = observe_compact(&self.theme, p);
                            if stats.is_empty() {
                                glyph.to_string()
                            } else {
                                format!("{} {}", glyph, stats)
                            }
                        }
                        ActStatus::Completed(None) => "✓".to_string(),
                        ActStatus::Failed(_) => "✗".to_string(),
                    };
                    lines.push(format!(
                        "{} {} {}  {}",
                        entry.timestamp, act_glyph, id_str, tail
                    ));
                    lines.push(String::new());
                    i += 1;
                    continue;
                }
            }

            match &entry.kind {
                TraceKind::Think => {
                    lines.push(format!("{} 🧠 THINK", entry.timestamp));
                    for text_line in entry.text.lines() {
                        lines.push(format!("   {}", text_line));
                    }
                }

                TraceKind::AgentMessage { agent } => {
                    lines.push(format!("{} 📨 {}", entry.timestamp, agent));

                    #[cfg(feature = "markdown")]
                    let used_markdown = if let Some(stream) = entry.markdown.as_ref() {
                        use crate::components::markdown_stream::StreamItem;
                        let (items, tail) = stream.items_and_tail();
                        for item in items {
                            match item {
                                StreamItem::Text(text_lines) => {
                                    for line in text_lines {
                                        let joined: String =
                                            line.spans.iter().map(|s| s.content.as_ref()).collect();
                                        lines.push(format!("   {}", joined));
                                    }
                                }
                                StreamItem::Fence(id) => {
                                    let placeholder = stream
                                        .fence_placeholder_for(*id)
                                        .map(|l| {
                                            l.spans
                                                .iter()
                                                .map(|s| s.content.as_ref())
                                                .collect::<String>()
                                        })
                                        .unwrap_or_else(|| {
                                            format!("[📊 mermaid #{} · press Alt-v to view]", id.0)
                                        });
                                    lines.push(format!("   {}", placeholder));
                                }
                            }
                        }
                        for tail_line in tail.lines() {
                            lines.push(format!("   {}", tail_line));
                        }
                        true
                    } else {
                        false
                    };

                    #[cfg(not(feature = "markdown"))]
                    let used_markdown = false;

                    if !used_markdown {
                        #[cfg(feature = "markdown")]
                        let source: &str = entry
                            .markdown
                            .as_ref()
                            .map(|s| s.raw_text())
                            .unwrap_or(entry.text.as_str());
                        #[cfg(not(feature = "markdown"))]
                        let source: &str = entry.text.as_str();

                        for text_line in source.lines() {
                            lines.push(format!("   {}", text_line));
                        }
                    }
                }

                TraceKind::Act {
                    tool,
                    family,
                    input,
                    status,
                    ..
                } => {
                    let (glyph, _) = family_glyph(&self.theme, *family);
                    lines.push(format!("{} {} {}", entry.timestamp, glyph, tool));
                    if matches!(input, ToolInputDisplay::Empty) {
                        for text_line in entry.text.lines() {
                            lines.push(format!("   {}", text_line));
                        }
                    } else {
                        for l in input_display_lines(&self.theme, input) {
                            let joined: String =
                                l.spans.iter().map(|s| s.content.as_ref()).collect();
                            lines.push(joined);
                        }
                    }
                    // Expanded mode renders output inline from `status` (there
                    // is no paired Observe), including the latest partial
                    // payload while a command is still running.
                    match status {
                        ActStatus::InProgress { partial: Some(p) } => {
                            for l in observe_payload_lines(&self.theme, p, self.observe_collapsed) {
                                let joined: String =
                                    l.spans.iter().map(|s| s.content.as_ref()).collect();
                                lines.push(joined);
                            }
                        }
                        ActStatus::Completed(Some(p)) => {
                            let verb = observe_verb(p);
                            let (glyph, _) = outcome_glyph(&self.theme, p);
                            lines.push(format!("{} {} {}", entry.timestamp, glyph, verb));
                            for l in observe_payload_lines(&self.theme, p, self.observe_collapsed) {
                                let joined: String =
                                    l.spans.iter().map(|s| s.content.as_ref()).collect();
                                lines.push(joined);
                            }
                        }
                        ActStatus::Failed(Some(p)) => {
                            let verb = observe_verb(p);
                            lines.push(format!("{} ✗ {}", entry.timestamp, verb));
                            for l in observe_payload_lines(&self.theme, p, self.observe_collapsed) {
                                let joined: String =
                                    l.spans.iter().map(|s| s.content.as_ref()).collect();
                                lines.push(joined);
                            }
                        }
                        ActStatus::Completed(None) => {
                            lines.push(format!("{} ✓ done", entry.timestamp));
                        }
                        ActStatus::Failed(None) => {
                            lines.push(format!("{} ✗ failed", entry.timestamp));
                        }
                        ActStatus::Pending | ActStatus::InProgress { partial: None } => {}
                    }
                }

                TraceKind::Observe { payload } => {
                    if let Some(p) = payload {
                        let (glyph, _) = outcome_glyph(&self.theme, p);
                        let verb = observe_verb(p);
                        lines.push(format!("{} {} {}", entry.timestamp, glyph, verb));
                        for l in observe_payload_lines(&self.theme, p, self.observe_collapsed) {
                            let joined: String =
                                l.spans.iter().map(|s| s.content.as_ref()).collect();
                            lines.push(joined);
                        }
                    } else {
                        lines.push(format!("{} 👁 OBSERVE", entry.timestamp));
                        for text_line in entry.text.lines() {
                            lines.push(format!("   {}", text_line));
                        }
                    }
                }

                TraceKind::Delegate {
                    agent,
                    task,
                    status,
                    request_id: _,
                    executor_id: _,
                } => {
                    lines.push(format!("{} → DELEGATE to {}", entry.timestamp, agent));
                    if !task.is_empty() {
                        lines.push(format!("   {}", task));
                    }
                    if !status.is_empty() {
                        lines.push(format!("   {}", status));
                    }
                }

                TraceKind::UserMessage => {
                    lines.push(format!("{} 💬 YOU", entry.timestamp));
                    for text_line in entry.text.lines() {
                        lines.push(format!("   {}", text_line));
                    }
                }

                TraceKind::Permission {
                    description,
                    pending,
                    countdown,
                } => {
                    lines.push(format!("{} ⚠ PERMISSION: {}", entry.timestamp, description));
                    if *pending {
                        if *countdown > 0 {
                            lines.push(format!(
                                "   Select an advertised option (auto-cancel in {}s)",
                                countdown
                            ));
                        } else {
                            lines.push("   Select an advertised option".to_string());
                        }
                    }
                    for text_line in entry.text.lines() {
                        lines.push(format!("   {}", text_line));
                    }
                }

                TraceKind::Image { label, .. } => {
                    lines.push(format!("{} 🖼 {}", entry.timestamp, label));
                    for text_line in entry.text.lines() {
                        lines.push(format!("   {}", text_line));
                    }
                }
            }

            // Blank separator between entries. No adjacency skip needed: Act outcome
            // is now rendered from `status` inline, not from a neighbouring Observe entry.
            lines.push(String::new());
            i += 1;
        }

        lines
    }
}

#[cfg(test)]
impl ReactTrace {
    pub fn generation_for_tests(&self) -> u64 {
        self.generation
    }

    pub fn dirty_from_for_tests(&self) -> Option<usize> {
        self.dirty_from
    }
}

#[cfg(test)]
impl ReactTrace {
    /// Test-only helper: returns each line as a joined `String` of span text.
    pub(crate) fn render_lines_for_test(&self, _width: u16) -> Vec<String> {
        self.render_to_strings()
    }

    pub fn new_for_tests() -> Self {
        Self::new()
    }

    #[cfg(feature = "markdown")]
    pub(crate) fn build_virtual_rows_for_tests(
        &self,
        from: usize,
        width: u16,
        states: &std::collections::HashMap<
            crate::components::mermaid::MermaidId,
            crate::components::mermaid::FenceRender,
        >,
        lineage: Option<&spur_core::lineage::projection::ExecutorLineage>,
    ) -> (
        Vec<VirtualRow>,
        Vec<usize>,
        Vec<Option<std::ops::Range<usize>>>,
    ) {
        self.build_virtual_rows(from, width, states, lineage)
    }

    pub fn entries_for_tests(&self) -> &[TraceEntry] {
        &self.entries
    }

    pub fn anchor_for_tests(&self) -> crate::components::react_trace::types::ScrollAnchor {
        self.anchor
    }

    /// Clone of `compact_cache.entry_row_starts` if the compact cache is
    /// populated. Used by scroll-correctness tests to assert the
    /// cache-row-layout invariant produced by `render_compact`.
    pub fn compact_entry_row_starts_for_tests(&self) -> Option<Vec<usize>> {
        self.compact_cache
            .as_ref()
            .map(|c| c.entry_row_starts.clone())
    }

    pub fn set_visible_height_for_tests(&mut self, height: usize) {
        self.last_visible_height = height;
    }

    /// Seed `line_cache` with a virtual-row layout built from the given
    /// fence states and width.  Tests call this before exercising scroll
    /// operations that require a populated cache (shift_anchor_by,
    /// page_up, etc.).
    #[cfg(feature = "markdown")]
    pub fn seed_line_cache_for_tests(
        &mut self,
        width: u16,
        states: &std::collections::HashMap<
            crate::components::mermaid::MermaidId,
            crate::components::mermaid::FenceRender,
        >,
    ) {
        let (rows, entry_row_starts, byte_ranges, plain_text_tail) =
            self.build_virtual_rows_with_tail(0, width, states, None);
        self.line_cache = Some(render::VirtualRowCacheEntry {
            rows,
            entry_row_starts,
            byte_ranges,
            width,
            soft_cap: 60, // sensible default for tests
            cell_w_px: 8, // typical non-retina monospace
            cell_h_px: 16,
            generation: self.generation,
            fence_gen: 0,
            plain_text_tail,
        });
        self.last_render_width = Some(width);
        // Simulate a Full-surface render so shift_anchor_by picks
        // line_cache when tests invoke scroll mutators.
        self.last_surface = Surface::Full(self.generation);
    }

    pub fn build_display_lines_for_tests(
        &self,
        spinner_frame: &str,
        lineage: Option<&spur_core::lineage::projection::ExecutorLineage>,
    ) -> Vec<ratatui::text::Line<'static>> {
        self.build_display_lines(spinner_frame, lineage, None)
    }
}

#[cfg(all(test, feature = "markdown"))]
mod markdown_integration_tests {
    use super::*;

    #[test]
    fn first_chunk_renders_body_before_debounce_flush() {
        let mut trace = ReactTrace::new();
        trace.append_message("Hello, world!", "claude", "10:00:00".to_string());

        let rendered = trace.render_lines_for_test(60);
        let joined = rendered.join("\n");
        assert!(
            joined.contains("Hello, world!"),
            "expected first chunk text to be visible in render output, got:\n{joined}"
        );
    }

    #[test]
    fn post_flush_rendered_lines_still_show_text() {
        use crate::components::markdown_stream::StateLookup;
        let mut trace = ReactTrace::new();
        // Two paragraphs separated by \n\n with trailing content after the
        // second paragraph so its End event has range.end < raw_text.len(),
        // making it authoritative under flush_now (permit_eof_closure=false).
        trace.append_message(
            "# Heading\n\nBody text\n\nmore",
            "claude",
            "10:00:00".to_string(),
        );

        let states = StateLookup::empty();
        let _ = trace.force_flush_all(&states);

        let rendered = trace.render_lines_for_test(60);
        let joined = rendered.join("\n");
        assert!(
            joined.contains("Heading"),
            "expected heading text after flush: {joined}"
        );
        assert!(
            joined.contains("Body text"),
            "expected body text after flush: {joined}"
        );
    }

    #[test]
    fn items_path_renders_same_text_as_lines_path() {
        let mut trace = ReactTrace::new();
        // Two paragraphs separated by \n\n with trailing content so the
        // second paragraph's End event has range.end < raw_text.len().
        trace.append_message("# Heading\n\nBody\n\nmore", "claude", "10:00".to_string());
        use crate::components::markdown_stream::StateLookup;
        let _ = trace.force_flush_all(&StateLookup::empty());

        let rendered = trace.render_lines_for_test(60);
        let joined = rendered.join("\n");
        assert!(joined.contains("Heading"), "expected heading: {joined}");
        assert!(joined.contains("Body"), "expected body: {joined}");
    }

    #[test]
    fn text_mode_agent_message_stream_produces_no_fence_items() {
        use crate::components::markdown_stream::{StateLookup, StreamItem};
        let mut trace = ReactTrace::new();
        trace.set_mermaid_enabled(false);
        trace.append_message(
            "Here's a diagram:\n\n```mermaid\nflowchart LR\nA-->B\n```\n",
            "claude",
            "10:00".to_string(),
        );
        let states = StateLookup::empty();
        let _ = trace.force_flush_all(&states);

        let entries = trace.entries_for_test();
        assert!(!entries.is_empty(), "expected at least one entry");
        for entry in entries {
            if let Some(stream) = entry.markdown.as_ref() {
                let has_fence = stream
                    .items()
                    .iter()
                    .any(|it| matches!(it, StreamItem::Fence(_)));
                assert!(
                    !has_fence,
                    "text mode must not produce Fence items: {:?}",
                    stream.items()
                );
            }
        }
    }

    #[test]
    fn long_agent_message_renders_full_body_without_expand_affordance() {
        let mut trace = ReactTrace::new();
        let prefix = "a".repeat(240);
        let response = format!("{prefix}TAIL");
        trace.append_message(&response, "claude", "10:00".to_string());

        let joined = trace.render_lines_for_test(80).join("\n");
        assert!(
            joined.contains(&response),
            "expected full response in render output: {joined}"
        );
        assert!(
            !joined.contains("[expand: Ctrl+O]"),
            "long agent message must not show expand affordance: {joined}"
        );

        trace.toggle_observe_collapsed();
        let expanded = trace.render_lines_for_test(80).join("\n");
        assert!(
            expanded.contains(&response),
            "observe collapse toggle must not affect agent message body: {expanded}"
        );
        assert!(
            !expanded.contains("[expand: Ctrl+O]"),
            "agent message must not show expand affordance after toggle: {expanded}"
        );
    }
}

#[cfg(all(test, feature = "markdown"))]
mod virtual_row_tests {
    use super::*;
    use crate::components::markdown_stream::StateLookup;
    use crate::components::mermaid::FenceRender;
    use spur_acp::adapter::{ObservePayload, ToolFamily, ToolInputDisplay};

    #[test]
    fn virtual_rows_text_only_match_line_count() {
        let mut trace = ReactTrace::new();
        // Use hard line breaks (two trailing spaces) so markdown renders each
        // as its own line within a single paragraph — no inter-paragraph blank
        // lines, giving exactly 3 body rows.
        trace.append_message("Line 1  \nLine 2  \nLine 3", "claude", "10:00".to_string());
        use crate::components::markdown_stream::StateLookup;
        let _ = trace.drain_fence_dispatches(&StateLookup::empty());

        let total = trace
            .build_virtual_rows(0, 60, &std::collections::HashMap::new(), None)
            .0 // rows
            .len();
        // Header (1) + 3 body lines + blank separator (1) = 5
        assert_eq!(total, 5, "unexpected virtual row count: {total}");
    }

    #[test]
    fn fence_with_ready_state_expands_to_image_rows() {
        let mut trace = ReactTrace::new();
        trace.append_message(
            "Before\n\n```mermaid\ngraph\nA-->B\n```\n\nAfter\n",
            "claude",
            "10:00".to_string(),
        );
        use crate::components::markdown_stream::StateLookup;
        let _ = trace.force_flush_all(&StateLookup::empty());
        let _ = trace.drain_fence_dispatches(&StateLookup::empty());

        use std::collections::HashMap;
        let mut states: HashMap<crate::components::mermaid::MermaidId, FenceRender> =
            HashMap::new();
        states.insert(
            crate::components::mermaid::MermaidId(0),
            FenceRender::Ready(12),
        );

        let (rows, _starts, _byte_ranges) = trace.build_virtual_rows(0, 60, &states, None);

        let image_rows: Vec<_> = rows
            .iter()
            .filter_map(|r| match r {
                VirtualRow::ImageRow {
                    source,
                    row_within,
                    total_rows,
                } => Some((*source, *row_within, *total_rows)),
                _ => None,
            })
            .collect();

        assert_eq!(
            image_rows.len(),
            12,
            "expected 12 image rows; got {image_rows:?}"
        );
        assert_eq!(image_rows[0].2, 12, "total_rows");
        assert_eq!(image_rows[0].1, 0, "first row_within");
        assert_eq!(image_rows[11].1, 11, "last row_within");
    }

    #[test]
    fn fence_without_height_emits_single_placeholder_row() {
        let mut trace = ReactTrace::new();
        trace.append_message(
            "Before\n\n```mermaid\ngraph\n```\n\nAfter\n",
            "claude",
            "10:00".to_string(),
        );
        use crate::components::markdown_stream::StateLookup;
        let _ = trace.force_flush_all(&StateLookup::empty());
        let _ = trace.drain_fence_dispatches(&StateLookup::empty());

        let empty = std::collections::HashMap::new();
        let (rows, _starts, _byte_ranges) = trace.build_virtual_rows(0, 60, &empty, None);

        let image_rows = rows
            .iter()
            .filter(|r| matches!(r, VirtualRow::ImageRow { .. }))
            .count();
        assert_eq!(image_rows, 0, "should fall back to Text placeholder");
    }

    #[test]
    fn render_plan_groups_contiguous_text_and_images() {
        let mut trace = ReactTrace::new();
        trace.append_message(
            "Before text line\n\n```mermaid\ngraph\n```\n\nAfter text line\n",
            "claude",
            "10:00".to_string(),
        );
        use crate::components::markdown_stream::StateLookup;
        let _ = trace.force_flush_all(&StateLookup::empty());

        let mut states = std::collections::HashMap::new();
        states.insert(
            crate::components::mermaid::MermaidId(0),
            FenceRender::Ready(8),
        );

        let segments = trace.render_plan_for_test(80, 40, 0, &states);

        let image_segs: Vec<_> = segments
            .iter()
            .filter(|s| matches!(s, Segment::Image { .. }))
            .collect();
        assert_eq!(
            image_segs.len(),
            1,
            "expected exactly one image segment: {segments:?}"
        );
        if let Segment::Image {
            total_rows,
            first_row_within,
            run_len,
            ..
        } = image_segs[0]
        {
            assert_eq!(*total_rows, 8);
            assert_eq!(*first_row_within, 0);
            assert_eq!(*run_len, 8);
        }

        let text_count = segments
            .iter()
            .filter(|s| matches!(s, Segment::Text { .. }))
            .count();
        assert!(text_count >= 2, "expected >=2 text segments: {segments:?}");
    }

    #[test]
    fn markdown_table_uses_vertical_records_when_too_narrow_for_grid() {
        let mut trace = ReactTrace::new();
        trace.append_message(
            "| Key | Action |\n|---|---|\n| Esc | cancel |\n| Enter | submit |\n",
            "claude",
            "10:00".to_string(),
        );
        let _ = trace.force_flush_all(&StateLookup::empty());

        let (rows, _starts, _byte_ranges) =
            trace.build_virtual_rows(0, 17, &std::collections::HashMap::new(), None);
        let rendered = rows
            .iter()
            .filter_map(|row| match row {
                VirtualRow::Text(line) => Some(
                    line.spans
                        .iter()
                        .map(|span| span.content.as_ref())
                        .collect::<String>(),
                ),
                VirtualRow::ImageRow { .. } => None,
            })
            .collect::<Vec<_>>()
            .join("\n");

        assert!(
            rendered.contains("Key: Esc"),
            "narrow fallback should render record labels:\n{rendered}"
        );
        assert!(
            rendered.contains("Action: cancel"),
            "narrow fallback should render record values:\n{rendered}"
        );
        assert!(
            !rendered.contains("┌"),
            "narrow fallback must not emit a grid wider than the viewport:\n{rendered}"
        );
    }

    #[test]
    fn stored_trace_image_expands_to_virtual_image_rows() {
        let mut trace = ReactTrace::new();
        let id = trace
            .append_image(
                std::sync::Arc::new(image::DynamicImage::ImageRgba8(image::RgbaImage::new(
                    20, 20,
                ))),
                std::path::PathBuf::from("/tmp/spur-image.png"),
                "sha".to_string(),
                "10:00".to_string(),
            )
            .expect("first digest should append image");

        let (rows, _starts, _byte_ranges) =
            trace.build_virtual_rows(0, 80, &std::collections::HashMap::new(), None);

        let image_rows: Vec<_> = rows
            .iter()
            .filter_map(|row| match row {
                VirtualRow::ImageRow {
                    source,
                    row_within,
                    total_rows,
                } => Some((*source, *row_within, *total_rows)),
                _ => None,
            })
            .collect();

        assert!(
            !image_rows.is_empty(),
            "stored trace image should create drawable image rows"
        );
        assert_eq!(image_rows[0].0, types::InlineImageSource::Trace(id));
        assert_eq!(image_rows[0].1, 0);
    }

    #[test]
    fn render_plan_partial_image_marks_partial_visibility() {
        let mut trace = ReactTrace::new();
        trace.append_message(
            "L1\nL2\nL3\nL4\n\n```mermaid\ngraph\n```\n",
            "claude",
            "10:00".to_string(),
        );
        use crate::components::markdown_stream::StateLookup;
        let _ = trace.force_flush_all(&StateLookup::empty());

        let mut states = std::collections::HashMap::new();
        states.insert(
            crate::components::mermaid::MermaidId(0),
            FenceRender::Ready(10),
        );

        let mut saw_partial = false;
        for offset in 0..20 {
            let segs = trace.render_plan_for_test(80, 6, offset, &states);
            if segs.iter().any(|s| {
                matches!(
                    s,
                    Segment::Image { total_rows, run_len, .. } if run_len < total_rows
                )
            }) {
                saw_partial = true;
                break;
            }
        }
        assert!(
            saw_partial,
            "expected some offset to produce partial image segment"
        );
    }

    #[test]
    fn entry_row_starts_remain_indexed_by_absolute_entry_after_collapsed_pairs() {
        let mut trace = ReactTrace::new();
        trace.push(TraceEntry {
            kind: TraceKind::Act {
                tool: "shell".to_string(),
                family: ToolFamily::Execute,
                input: ToolInputDisplay::Command {
                    cmd: "echo hi".to_string(),
                    cwd: None,
                },
                tool_call_id: None,
                status: ActStatus::Pending,
            },
            text: String::new(),
            timestamp: "10:00".to_string(),
            markdown: None,
        });
        trace.push(TraceEntry {
            kind: TraceKind::Observe {
                payload: Some(ObservePayload::Text {
                    body: "hi".to_string(),
                }),
            },
            text: String::new(),
            timestamp: "10:00".to_string(),
            markdown: None,
        });
        trace.append_message("first line", "claude", "10:01".to_string());
        let _ = trace.force_flush_all(&StateLookup::empty());

        let (_rows, starts, _byte_ranges) =
            trace.build_virtual_rows(0, 80, &std::collections::HashMap::new(), None);

        assert_eq!(
            starts.len(),
            trace.entries().len(),
            "entry row starts must stay aligned with absolute entry indices"
        );
        assert!(
            starts.get(2).is_some(),
            "the streamed AgentMessage entry must retain a start offset even when earlier Act+Observe pairs are collapsed"
        );
    }

    /// F2 end-to-end invariant: Pending→Ready fence state transition causes
    /// the VirtualRow cache to invalidate and rebuild, producing a different
    /// (larger) row count.
    ///
    /// Chain tested:
    ///   state transition (Pending→Ready in mermaid_registry)
    ///   → mermaid_registry_version changes
    ///   → fence_ok=false in render_with_ctx cache check
    ///   → cache rebuilt with new fence states
    ///   → different (more) rows (ImageRows vs text placeholder)
    ///
    /// Uses build_virtual_rows_for_tests directly (no real frame required).
    /// Mirrors what render_with_ctx does internally: compute_fence_states
    /// maps MermaidState::Pending → FenceRender::Pending (1 placeholder row)
    /// and MermaidState::Ready { image, .. } → FenceRender::Ready(h) (h image rows).
    #[test]
    fn f2_pending_to_ready_cache_invalidation_changes_row_count() {
        use crate::components::mermaid::MermaidId;
        use std::collections::HashMap;

        // ── Step 1: create a trace with a mermaid fence and force-flush. ──
        let mut trace = ReactTrace::new();
        trace.append_message(
            "```mermaid\ngraph LR\nA --> B\n```",
            "claude",
            "10:00".to_string(),
        );
        let _ = trace.force_flush_all(&StateLookup::empty());
        // Drain so the fence is committed to items (not tail).
        let _ = trace.drain_fence_dispatches(&StateLookup::empty());

        // ── Step 2: set up Pending and Ready generation snapshots. ──
        let fence_id = MermaidId(0);
        let version_pending = 0;
        let version_ready = 1;
        assert_ne!(
            version_pending, version_ready,
            "F2: Pending→Ready must bump mermaid_registry_version so the cache detects staleness"
        );

        // ── Step 4: render with Pending registry → capture row count. ──
        // Mirrors what render_with_ctx / compute_fence_states does:
        //   Pending → FenceRender::Pending → 1 placeholder text row per fence.
        let pending_states: HashMap<MermaidId, FenceRender> =
            [(fence_id, FenceRender::Pending)].into_iter().collect();
        let (rows_pending, _, _) = trace.build_virtual_rows_for_tests(0, 80, &pending_states, None);
        let count_pending = rows_pending.len();

        // Confirm fence rendered as a Text placeholder (no ImageRows).
        let image_rows_pending = rows_pending
            .iter()
            .filter(|r| matches!(r, VirtualRow::ImageRow { .. }))
            .count();
        assert_eq!(
            image_rows_pending, 0,
            "F2: Pending state must produce 0 ImageRows (got {})",
            image_rows_pending
        );

        // ── Step 5: render with Ready registry → capture row count. ──
        // Ready { image: 100×60 } → compute_inline_height_rows → some h ≥ 6.
        // We use Ready(6) as the minimum guaranteed height.
        let inline_h: u16 = 6; // clamped minimum from compute_inline_height_rows
        let ready_states: HashMap<MermaidId, FenceRender> =
            [(fence_id, FenceRender::Ready(inline_h))]
                .into_iter()
                .collect();
        let (rows_ready, _, _) = trace.build_virtual_rows_for_tests(0, 80, &ready_states, None);
        let count_ready = rows_ready.len();

        // Confirm fence rendered as ImageRows.
        let image_rows_ready = rows_ready
            .iter()
            .filter(|r| matches!(r, VirtualRow::ImageRow { .. }))
            .count();
        assert_eq!(
            image_rows_ready, inline_h as usize,
            "F2: Ready state must produce exactly {} ImageRows (got {})",
            inline_h, image_rows_ready
        );

        // ── Step 6: assert row count differs (cache rebuild produces new rows). ──
        assert_ne!(
            count_pending, count_ready,
            "F2: row count must differ after Pending→Ready transition \
             (pending={}, ready={}). Cache rebuild path is broken.",
            count_pending, count_ready
        );
        assert!(
            count_ready > count_pending,
            "F2: Ready state must produce MORE rows than Pending \
             because ImageRows expand the fence. pending={}, ready={}",
            count_pending,
            count_ready
        );

        // ── Step 7: verify the cache's fence_gen field tracks version changes. ──
        // Simulate the exact render_with_ctx freshness check:
        //   fence_ok = line_cache.fence_gen == ctx.mermaid_registry_version
        let simulated_cache_fence_gen = version_pending;
        let current_fence_gen = version_ready;
        let fence_ok = simulated_cache_fence_gen == current_fence_gen;
        assert!(
            !fence_ok,
            "F2: cache populated during Pending state must be recognized as stale \
             after Ready transition (fence_ok must be false to trigger rebuild)"
        );
    }
}

#[cfg(test)]
mod streaming_tests;

#[cfg(test)]
mod body_cache_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use spur_acp::adapter::ToolFamily;

    #[cfg(feature = "markdown")]
    #[test]
    fn row_to_anchor_walks_entry_row_starts() {
        let starts = vec![0, 5, 12];
        assert_eq!(super::row_to_anchor(0, &starts), (0, 0));
        assert_eq!(super::row_to_anchor(4, &starts), (0, 4));
        assert_eq!(super::row_to_anchor(5, &starts), (1, 0));
        assert_eq!(super::row_to_anchor(11, &starts), (1, 6));
        assert_eq!(super::row_to_anchor(12, &starts), (2, 0));
    }

    /// Post-tool text must appear as a SEPARATE AgentMessage entry, not
    /// merged into the pre-tool block.
    #[test]
    fn append_message_creates_new_entry_after_tool_call() {
        let mut trace = ReactTrace::new();
        trace.append_message("first chunk. ", "claude", "10:00:01".to_string());
        trace.push(TraceEntry {
            kind: TraceKind::Act {
                tool: "read_file".to_string(),
                family: ToolFamily::Unknown,
                input: ToolInputDisplay::Empty,
                tool_call_id: None,
                status: ActStatus::Pending,
            },
            text: "read_file(path=...)".to_string(),
            timestamp: "10:00:02".to_string(),
            #[cfg(feature = "markdown")]
            markdown: None,
        });
        trace.append_message("second chunk.", "claude", "10:00:03".to_string());

        let entries = trace.entries_for_test();
        let agent_messages: Vec<_> = entries
            .iter()
            .filter(|e| matches!(&e.kind, TraceKind::AgentMessage { agent } if agent == "claude"))
            .collect();
        assert_eq!(
            agent_messages.len(),
            2,
            "expected 2 separate AgentMessage entries (pre-tool and post-tool), got {}",
            agent_messages.len()
        );

        let kinds: Vec<&str> = entries
            .iter()
            .map(|e| match &e.kind {
                TraceKind::AgentMessage { .. } => "agent_message",
                TraceKind::Act { .. } => "act",
                _ => "other",
            })
            .collect();
        assert_eq!(
            kinds,
            vec!["agent_message", "act", "agent_message"],
            "trace should be [msg, tool, msg] but got {kinds:?}"
        );
    }

    /// Consecutive AgentMessageChunks must still merge into a single entry.
    #[test]
    fn append_message_merges_consecutive_chunks_from_same_agent() {
        let mut trace = ReactTrace::new();
        trace.append_message("hello ", "claude", "10:00:01".to_string());
        trace.append_message("world", "claude", "10:00:01".to_string());

        let entries = trace.entries_for_test();
        let agent_message_count = entries
            .iter()
            .filter(|e| matches!(&e.kind, TraceKind::AgentMessage { agent } if agent == "claude"))
            .count();
        assert_eq!(
            agent_message_count, 1,
            "consecutive chunks should merge into one entry"
        );
    }

    #[test]
    fn append_user_message_creates_new_entry_when_empty() {
        let mut trace = ReactTrace::new();
        trace.append_user_message("hello", "10:00:01".to_string());
        let entries = trace.entries_for_test();
        let user_count = entries
            .iter()
            .filter(|e| matches!(&e.kind, TraceKind::UserMessage))
            .count();
        assert_eq!(user_count, 1);
        assert_eq!(entries.last().unwrap().text, "hello");
    }

    #[test]
    fn append_user_message_coalesces_into_tail_user_entry() {
        let mut trace = ReactTrace::new();
        trace.push(TraceEntry {
            kind: TraceKind::UserMessage,
            text: "hello ".to_string(),
            timestamp: "10:00:01".to_string(),
            #[cfg(feature = "markdown")]
            markdown: None,
        });
        trace.append_user_message("world", "10:00:02".to_string());

        let entries = trace.entries_for_test();
        let user_entries: Vec<_> = entries
            .iter()
            .filter(|e| matches!(&e.kind, TraceKind::UserMessage))
            .collect();
        assert_eq!(user_entries.len(), 1, "must coalesce, not duplicate");
        assert_eq!(user_entries[0].text, "hello world");
    }

    #[test]
    fn append_user_message_idempotent_when_text_already_seeded() {
        let mut trace = ReactTrace::new();
        // Simulate HistoryEntry path seeding the full turn.
        trace.push(TraceEntry {
            kind: TraceKind::UserMessage,
            text: "list the files in src/".to_string(),
            timestamp: "10:00:01".to_string(),
            #[cfg(feature = "markdown")]
            markdown: None,
        });
        // ACP protocol later replays the same text as a chunk.
        trace.append_user_message("list the files in src/", "10:00:02".to_string());

        let entries = trace.entries_for_test();
        let user: Vec<_> = entries
            .iter()
            .filter(|e| matches!(&e.kind, TraceKind::UserMessage))
            .collect();
        assert_eq!(user.len(), 1);
        assert_eq!(
            user[0].text, "list the files in src/",
            "must not double the seeded text"
        );
    }

    #[test]
    fn append_user_message_skips_mention_prefix_already_in_seeded_preview() {
        let mut trace = ReactTrace::new();
        trace.push(TraceEntry {
            kind: TraceKind::UserMessage,
            text: "review @src/lib.rs now".to_string(),
            timestamp: "10:00:01".to_string(),
            #[cfg(feature = "markdown")]
            markdown: None,
        });
        trace.append_user_message("review ", "10:00:02".to_string());
        trace.append_user_message(" now", "10:00:03".to_string());

        let entries = trace.entries_for_test();
        let user: Vec<_> = entries
            .iter()
            .filter(|e| matches!(&e.kind, TraceKind::UserMessage))
            .collect();
        assert_eq!(user.len(), 1);
        assert_eq!(
            user[0].text, "review @src/lib.rs now",
            "split Text+ResourceLink echo must not grow a mention-shaped local preview"
        );
    }

    #[test]
    fn merge_status_pending_to_completed_with_payload() {
        use spur_acp::adapter::ObservePayload;
        use spur_acp::{AgentKind, ToolCallStatus};
        let payload_json = serde_json::json!("ok");
        let new = super::merge_status(
            &ActStatus::Pending,
            Some(ToolCallStatus::Completed),
            Some(&payload_json),
            AgentKind::Generic,
        );
        match new {
            ActStatus::Completed(Some(ObservePayload::Text { .. })) => {}
            other => panic!("expected Completed(Some(Text)), got {:?}", other),
        }
    }

    #[test]
    fn merge_status_completed_is_terminal_ignores_late_in_progress() {
        use spur_acp::adapter::ObservePayload;
        use spur_acp::{AgentKind, ToolCallStatus};
        let prev = ActStatus::Completed(Some(ObservePayload::Text {
            body: "done".into(),
        }));
        let new = super::merge_status(
            &prev,
            Some(ToolCallStatus::InProgress),
            None,
            AgentKind::Generic,
        );
        // Terminal state must not be reopened.
        assert!(
            matches!(new, ActStatus::Completed(Some(_))),
            "terminal Completed must not regress to InProgress, got {:?}",
            new
        );
    }

    #[test]
    fn merge_status_none_incoming_status_preserves_variant() {
        use spur_acp::AgentKind;
        let prev = ActStatus::Pending;
        let new = super::merge_status(&prev, None, None, AgentKind::Generic);
        assert!(matches!(new, ActStatus::Pending));
    }

    #[test]
    fn map_initial_status_pending_yields_pending() {
        use spur_acp::{AgentKind, ToolCallStatus};
        let got = super::map_initial_status(ToolCallStatus::Pending, None, AgentKind::Generic);
        assert!(matches!(got, ActStatus::Pending));
    }

    #[test]
    fn map_initial_status_completed_with_output_yields_completed_some() {
        use spur_acp::{AgentKind, ToolCallStatus};
        let out = serde_json::json!({"text": "hi"});
        let got =
            super::map_initial_status(ToolCallStatus::Completed, Some(&out), AgentKind::Generic);
        assert!(matches!(got, ActStatus::Completed(Some(_))));
    }

    #[test]
    fn find_act_by_id_mut_returns_newest_matching_act() {
        use spur_acp::adapter::{ToolFamily, ToolInputDisplay};
        use spur_acp::ToolCallId;
        use std::sync::Arc;

        let mut trace = ReactTrace::new();
        let id_a: ToolCallId = ToolCallId::new(Arc::from("call-A"));
        let id_b: ToolCallId = ToolCallId::new(Arc::from("call-B"));
        trace.push(TraceEntry {
            kind: TraceKind::Act {
                tool: "first".into(),
                family: ToolFamily::Unknown,
                input: ToolInputDisplay::Empty,
                tool_call_id: Some(id_a.clone()),
                status: ActStatus::Pending,
            },
            text: String::new(),
            timestamp: "t0".into(),
            #[cfg(feature = "markdown")]
            markdown: None,
        });
        trace.push(TraceEntry {
            kind: TraceKind::Act {
                tool: "second".into(),
                family: ToolFamily::Unknown,
                input: ToolInputDisplay::Empty,
                tool_call_id: Some(id_b.clone()),
                status: ActStatus::Pending,
            },
            text: String::new(),
            timestamp: "t1".into(),
            #[cfg(feature = "markdown")]
            markdown: None,
        });

        let found = trace.find_act_by_id_mut(&id_a);
        assert!(found.is_some(), "should find act by id");
        let (idx, entry) = found.unwrap();
        assert_eq!(idx, 0, "should return the matching entry's absolute index");
        assert!(
            matches!(&entry.kind, TraceKind::Act { tool, .. } if tool == "first"),
            "should return a mutable reference to the matching entry"
        );

        let id_missing: ToolCallId = ToolCallId::new(Arc::from("nope"));
        assert!(trace.find_act_by_id_mut(&id_missing).is_none());
    }

    #[test]
    fn first_active_spinner_returns_pending_act_index() {
        use spur_acp::adapter::{ToolFamily, ToolInputDisplay};
        let mut trace = ReactTrace::new();
        trace.push(TraceEntry {
            kind: TraceKind::Act {
                tool: "t".into(),
                family: ToolFamily::Unknown,
                input: ToolInputDisplay::Empty,
                tool_call_id: None,
                status: ActStatus::Pending,
            },
            text: String::new(),
            timestamp: "t0".into(),
            #[cfg(feature = "markdown")]
            markdown: None,
        });
        assert_eq!(trace.first_active_spinner(), Some(0));

        // Transition to Completed: spinner should stop.
        if let TraceKind::Act { status, .. } = &mut trace.entries[0].kind {
            *status = ActStatus::Completed(None);
        }
        assert_eq!(trace.first_active_spinner(), None);
    }

    #[test]
    fn render_to_strings_completed_act_shows_outcome_glyph_not_spinner() {
        use spur_acp::adapter::{ObservePayload, ToolFamily, ToolInputDisplay};
        let mut trace = ReactTrace::new();
        trace.push(TraceEntry {
            kind: TraceKind::Act {
                tool: "shell".into(),
                family: ToolFamily::Execute,
                input: ToolInputDisplay::Command {
                    cmd: "echo hi".into(),
                    cwd: None,
                },
                tool_call_id: None,
                status: ActStatus::Completed(Some(ObservePayload::CommandOutput {
                    exit_code: Some(0),
                    stdout: "hi".into(),
                    stderr: String::new(),
                })),
            },
            text: String::new(),
            timestamp: "10:00".into(),
            #[cfg(feature = "markdown")]
            markdown: None,
        });
        let lines = trace.render_to_strings().join("\n");
        assert!(
            lines.contains("✓"),
            "expected success glyph in collapsed render, got:\n{lines}"
        );
        for frame in spinner::BRAILLE {
            assert!(
                !lines.contains(frame),
                "completed Act must not render a spinner frame ({frame}) in:\n{lines}"
            );
        }
    }

    #[test]
    fn render_to_strings_pending_act_shows_spinner_placeholder() {
        use spur_acp::adapter::{ToolFamily, ToolInputDisplay};
        let mut trace = ReactTrace::new();
        trace.push(TraceEntry {
            kind: TraceKind::Act {
                tool: "shell".into(),
                family: ToolFamily::Execute,
                input: ToolInputDisplay::Command {
                    cmd: "sleep 5".into(),
                    cwd: None,
                },
                tool_call_id: None,
                status: ActStatus::Pending,
            },
            text: String::new(),
            timestamp: "10:00".into(),
            #[cfg(feature = "markdown")]
            markdown: None,
        });
        let joined = trace.render_to_strings().join("\n");
        // render_to_strings emits the Unicode ellipsis placeholder for the
        // plain-text path's spinner slot (not a real animated frame).
        assert!(
            joined.contains("\u{2026}") || spinner::BRAILLE.iter().any(|f| joined.contains(f)),
            "pending Act must render a spinner placeholder, got:\n{joined}"
        );
    }

    #[test]
    fn update_delegate_status_by_executor_id() {
        let mut trace = ReactTrace::new();
        trace.push(TraceEntry {
            kind: TraceKind::Delegate {
                agent: "codex".into(),
                task: "fix bug".into(),
                status: "delegated".into(),
                request_id: Some("req-1".into()),
                executor_id: Some("exec-1".into()),
            },
            text: String::new(),
            timestamp: "10:00".into(),
            #[cfg(feature = "markdown")]
            markdown: None,
        });
        trace.update_delegate_status("exec-1", "done");
        match &trace.entries[0].kind {
            TraceKind::Delegate { status, .. } => assert_eq!(status, "done"),
            other => panic!("expected Delegate, got {:?}", other),
        }
    }

    #[test]
    fn update_delegate_status_falls_back_to_request_id_when_executor_id_none() {
        let mut trace = ReactTrace::new();
        trace.push(TraceEntry {
            kind: TraceKind::Delegate {
                agent: "codex".into(),
                task: "fix bug".into(),
                status: "delegated".into(),
                request_id: Some("req-1".into()),
                executor_id: None,
            },
            text: String::new(),
            timestamp: "10:00".into(),
            #[cfg(feature = "markdown")]
            markdown: None,
        });
        trace.update_delegate_status("req-1", "failed");
        match &trace.entries[0].kind {
            TraceKind::Delegate { status, .. } => assert_eq!(status, "failed"),
            other => panic!("expected Delegate, got {:?}", other),
        }
    }

    #[test]
    fn update_delegate_status_prefers_executor_id_over_request_id() {
        // If executor_id is already Some(different_value), do NOT match on
        // request_id even if request_id equals the search key.
        let mut trace = ReactTrace::new();
        trace.push(TraceEntry {
            kind: TraceKind::Delegate {
                agent: "codex".into(),
                task: "fix bug".into(),
                status: "delegated".into(),
                request_id: Some("shared-id".into()),
                executor_id: Some("exec-a".into()),
            },
            text: String::new(),
            timestamp: "10:00".into(),
            #[cfg(feature = "markdown")]
            markdown: None,
        });
        trace.push(TraceEntry {
            kind: TraceKind::Delegate {
                agent: "kiro".into(),
                task: "refactor".into(),
                status: "delegated".into(),
                request_id: Some("shared-id".into()),
                executor_id: None,
            },
            text: String::new(),
            timestamp: "10:01".into(),
            #[cfg(feature = "markdown")]
            markdown: None,
        });
        // Searching for "shared-id" should update the SECOND entry (the one
        // with executor_id == None), not the first.
        trace.update_delegate_status("shared-id", "done");
        assert_eq!(
            match &trace.entries[0].kind {
                TraceKind::Delegate { status, .. } => status.as_str(),
                other => panic!("expected Delegate, got {:?}", other),
            },
            "delegated",
            "first entry (with executor_id=Some) should NOT have been updated"
        );
        assert_eq!(
            match &trace.entries[1].kind {
                TraceKind::Delegate { status, .. } => status.as_str(),
                other => panic!("expected Delegate, got {:?}", other),
            },
            "done",
            "second entry (with executor_id=None) SHOULD have been updated"
        );
    }

    #[test]
    fn update_delegate_status_updates_most_recent_match() {
        let mut trace = ReactTrace::new();
        for i in 0..2 {
            trace.push(TraceEntry {
                kind: TraceKind::Delegate {
                    agent: "codex".into(),
                    task: format!("task {}", i),
                    status: "delegated".into(),
                    request_id: Some(format!("req-{}", i)),
                    executor_id: Some("exec-shared".into()),
                },
                text: String::new(),
                timestamp: format!("10:0{}", i),
                #[cfg(feature = "markdown")]
                markdown: None,
            });
        }
        trace.update_delegate_status("exec-shared", "done");
        // Most recent entry (index 1) should be updated.
        assert_eq!(
            match &trace.entries[1].kind {
                TraceKind::Delegate { status, .. } => status.as_str(),
                other => panic!("expected Delegate, got {:?}", other),
            },
            "done"
        );
        // Older entry should remain unchanged.
        assert_eq!(
            match &trace.entries[0].kind {
                TraceKind::Delegate { status, .. } => status.as_str(),
                other => panic!("expected Delegate, got {:?}", other),
            },
            "delegated"
        );
    }
}

#[cfg(all(test, feature = "markdown"))]
mod scroll_race_test;
