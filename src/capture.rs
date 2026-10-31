//! One-use capture ledger (inventory §3.1–§3.2) — cua's `capture_registry.rs`
//! semantics, adapted to the registry chokepoint.
//!
//! A **capture** is what one look produced: a screenshot, an accessibility
//! tree, the geometry/revision they were taken against. A capture mints a
//! [`CaptureId`] that authorizes **exactly one action**; the second attempt
//! under the same capture is refused ("reuse denied"). That is the product
//! rule the P3 acceptance names: *"one screenshot's `capture_id` may be
//! consumed by one action once; reuse is refused."*
//!
//! Why a ledger and not a counter (inventory §3.2): authority must be tied to
//! **what the capture saw** — geometry, observation revision — so "coordinates
//! from an old screenshot" cannot land on a new layout unnoticed. The binding
//! is pack-supplied and opaque to this layer (the pack knows what a revision
//! is); the ledger's job is the one-use bookkeeping around it.
//!
//! What is copied from cua, and what is not:
//!
//! | cua (`capture_registry.rs`) | here |
//! |---|---|
//! | `publish` → `CaptureId` | [`CaptureLedger::mint`] |
//! | `admit_action` consumes atomically ("one action attempt") | [`CaptureLedger::consume`] — consumes the **attempt**, never refunded |
//! | binding = runtime/session generation + target + native frame | binding = the pack's opaque descriptor (revision/geometry) |
//! | tombstones for externally presented ids | `last_consumed` record — ids are never presented back (see below) |
//! | id = `capture_<uuid16>_<seq16>` | `capture_<face>_<seq>` — face + mint order, readable in a journal row |
//!
//! cua tombstones spent ids because callers *present* them; here the engine
//! pops the face's live capture at the chokepoint, so the only handle is the
//! face slot and the spent record answers "already consumed by …". Minting is
//! likewise engine-side: a pack returns a `CAPTURE_KEY` descriptor and the
//! chokepoint stamps `CAPTURE_ID_KEY` back — the model sees the id for
//! correlation, never threads it.

use serde_json::Value;
use std::collections::HashMap;
use std::fmt;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Pack → engine: the return key carrying what this capture bound
/// (revision/geometry — opaque to the engine).
pub const CAPTURE_KEY: &str = "capture";

/// Engine → model/audit: the return key stamped with the minted [`CaptureId`].
pub const CAPTURE_ID_KEY: &str = "capture_id";

/// Default look→act window: a capture older than this refuses to authorize.
/// UI work is seconds-to-minutes; ten minutes is already generous and keeps a
/// forgotten session from authorizing a click on a long-gone layout.
pub const DEFAULT_CAPTURE_TTL: Duration = Duration::from_secs(600);

/// How one method relates to captures — the `MethodSpec` capture column.
///
/// Declared per method and enforced at the chokepoint (`declared =
/// enforceable`): a `Consume` method without a live capture never reaches the
/// wire; only a `Mint` method's return may mint.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CaptureUse {
    /// No capture relation: pure computation, reads, blind input that carries
    /// no capture-derived target.
    #[default]
    None,
    /// Produces a capture (a look: `m.observe`, `desk.shot`, `desk.win_state`).
    /// A return carrying [`CAPTURE_KEY`] mints one id on this method's face.
    Mint,
    /// Acts on a target drawn from a capture — the call must consume one
    /// (`m.click`, `desk.drag`, …). One action attempt per capture.
    Consume,
}

/// One minted capture authority: face + mint order.
///
/// Displayed as `capture_<face>_<seq>` (`capture_m_0001`) so a journal row
/// reads back which face and which look authorized the action.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct CaptureId {
    face: String,
    seq: u64,
}

impl CaptureId {
    /// The face this capture belongs to (`"m"`, `"desk"`).
    pub fn face(&self) -> &str {
        &self.face
    }

    /// Mint order within the face.
    pub fn seq(&self) -> u64 {
        self.seq
    }
}

impl fmt::Debug for CaptureId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl fmt::Display for CaptureId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "capture_{}_{:04}", self.face, self.seq)
    }
}

/// A capture consumed by (or refused to) one action attempt.
#[derive(Debug, Clone)]
pub struct ConsumedCapture {
    /// The authority spent on this attempt.
    pub id: CaptureId,
    /// The look that minted it (`"m.observe"`).
    pub minted_by: String,
    /// What the capture bound (pack-supplied; opaque here).
    pub binding: Value,
}

/// Why a capture gate refused — structured, never a silent fall-through.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaptureRefusal {
    /// The face has no live capture: nothing was looked at (or the last look
    /// expired and was dropped). The model's fix is a fresh look.
    NoLiveCapture {
        /// Face that needed a capture.
        face: String,
    },
    /// The one-use rule (inventory §3.1): this capture already authorized an
    /// action attempt. Reuse denied — look again.
    AlreadyConsumed {
        /// The spent capture.
        id: CaptureId,
        /// The method that spent it.
        by: String,
    },
    /// The live capture aged out ([`CaptureLedger::with_ttl`]) before use.
    Expired {
        /// The capture that aged out.
        id: CaptureId,
    },
}

impl CaptureRefusal {
    /// The capture this refusal is about, when one is identifiable.
    pub fn id(&self) -> Option<&CaptureId> {
        match self {
            Self::NoLiveCapture { .. } => None,
            Self::AlreadyConsumed { id, .. } | Self::Expired { id } => Some(id),
        }
    }

    /// Human/model-facing reason — the fix ("look again") is in every variant.
    pub fn reason(&self) -> String {
        match self {
            Self::NoLiveCapture { face } => {
                format!("no live capture on face `{face}` — look again before acting")
            }
            Self::AlreadyConsumed { id, by } => {
                format!("{id} was already consumed by {by} — look again before acting")
            }
            Self::Expired { id } => format!("{id} expired before use — look again before acting"),
        }
    }
}

impl fmt::Display for CaptureRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.reason())
    }
}

/// One face's bookkeeping: the live capture plus the last spent one.
#[derive(Debug)]
struct FaceState {
    /// Mint order within the face.
    seq: u64,
    /// The capture currently authorizing actions (a newer mint supersedes).
    live: Option<StoredCapture>,
    /// Last consumed record — answers "reuse denied" with the spent id.
    last_consumed: Option<(CaptureId, String, Instant)>,
}

#[derive(Debug)]
struct StoredCapture {
    id: CaptureId,
    minted_by: String,
    binding: Value,
    minted_at: Instant,
}

/// The ledger: one live capture per face, one action attempt per capture.
///
/// Owned by the host chokepoint (codexec's registry / the gate) — host-side,
/// so a stub restart cannot lose it and no code path to the device skips it
/// (inventory §3.4: the host ledger is the audit layer a device-side gate
/// cannot replace).
#[derive(Debug)]
pub struct CaptureLedger {
    ttl: Duration,
    faces: Mutex<HashMap<String, FaceState>>,
}

impl Default for CaptureLedger {
    fn default() -> Self {
        Self::new()
    }
}

impl CaptureLedger {
    /// A ledger with [`DEFAULT_CAPTURE_TTL`].
    pub fn new() -> Self {
        Self::with_ttl(DEFAULT_CAPTURE_TTL)
    }

    /// A ledger with an explicit look→act window (`Duration::ZERO` expires
    /// immediately — used by tests).
    pub fn with_ttl(ttl: Duration) -> Self {
        Self {
            ttl,
            faces: Mutex::new(HashMap::new()),
        }
    }

    /// Mint the authority for one look. A previous live capture on the face is
    /// superseded (dropped): implicit consumption means the model always acts
    /// on the latest look.
    pub fn mint(&self, face: &str, minted_by: &str, binding: Value) -> CaptureId {
        let mut faces = self.faces.lock().unwrap_or_else(|p| p.into_inner());
        let state = faces.entry(face.to_string()).or_insert(FaceState {
            seq: 0,
            live: None,
            last_consumed: None,
        });
        state.seq += 1;
        let id = CaptureId {
            face: face.to_string(),
            seq: state.seq,
        };
        state.live = Some(StoredCapture {
            id: id.clone(),
            minted_by: minted_by.to_string(),
            binding,
            minted_at: Instant::now(),
        });
        id
    }

    /// Spend the face's live capture on **one action attempt** — atomic
    /// remove-and-return, exactly like cua's `admit_action`. Attempts that
    /// later fail their own contract keep the capture spent: one look
    /// authorizes one attempt, not one success.
    pub fn consume(&self, face: &str, by: &str) -> Result<ConsumedCapture, CaptureRefusal> {
        let mut faces = self.faces.lock().unwrap_or_else(|p| p.into_inner());
        let Some(state) = faces.get_mut(face) else {
            return Err(CaptureRefusal::NoLiveCapture {
                face: face.to_string(),
            });
        };
        let Some(live) = state.live.take() else {
            return Err(match &state.last_consumed {
                Some((id, by, _)) => CaptureRefusal::AlreadyConsumed {
                    id: id.clone(),
                    by: by.clone(),
                },
                None => CaptureRefusal::NoLiveCapture {
                    face: face.to_string(),
                },
            });
        };
        if live.minted_at.elapsed() >= self.ttl {
            return Err(CaptureRefusal::Expired { id: live.id });
        }
        state.last_consumed = Some((live.id.clone(), by.to_string(), Instant::now()));
        Ok(ConsumedCapture {
            id: live.id,
            minted_by: live.minted_by,
            binding: live.binding,
        })
    }
}

/// The face a method belongs to: `"m.observe"` → `"m"`.
///
/// Faces are namespace roots, so one pack's looks authorize only that pack's
/// actions (a phone capture never authorizes a desktop click).
pub fn face_of(method: &str) -> &str {
    method.split('.').next().unwrap_or(method)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn one_capture_authorizes_one_attempt_and_reuse_is_refused() {
        let ledger = CaptureLedger::new();
        let id = ledger.mint("m", "m.observe", json!({"revision": "7"}));
        assert_eq!(id.to_string(), "capture_m_0001");

        let spent = ledger.consume("m", "m.click").expect("first attempt");
        assert_eq!(spent.id, id);
        assert_eq!(spent.minted_by, "m.observe");
        assert_eq!(spent.binding, json!({"revision": "7"}));

        let refusal = ledger.consume("m", "m.click").unwrap_err();
        assert_eq!(
            refusal,
            CaptureRefusal::AlreadyConsumed {
                id,
                by: "m.click".into(),
            }
        );
        assert!(refusal.reason().contains("look again"));
        assert_eq!(
            refusal.id().map(|i| i.to_string()).as_deref(),
            Some("capture_m_0001")
        );
    }

    #[test]
    fn a_fresh_look_restores_the_authority() {
        let ledger = CaptureLedger::new();
        ledger.mint("m", "m.observe", json!({}));
        ledger.consume("m", "m.click").expect("first");
        let second = ledger.mint("m", "m.observe", json!({}));
        let again = ledger.consume("m", "m.click").expect("second look");
        assert_eq!(again.id, second);
        assert_eq!(second.to_string(), "capture_m_0002");
    }

    #[test]
    fn a_newer_look_supersedes_the_old_one() {
        let ledger = CaptureLedger::new();
        let first = ledger.mint("m", "m.observe", json!({"revision": "1"}));
        let second = ledger.mint("m", "m.observe", json!({"revision": "2"}));
        let spent = ledger.consume("m", "m.click").expect("latest look wins");
        assert_eq!(spent.id, second);
        assert_ne!(spent.id, first);
    }

    #[test]
    fn faces_keep_their_own_authority() {
        let ledger = CaptureLedger::new();
        let phone = ledger.mint("m", "m.observe", json!({}));
        let desk = ledger.mint("desk", "desk.shot", json!({}));
        assert_eq!(ledger.consume("m", "m.click").unwrap().id, phone);
        assert_eq!(ledger.consume("desk", "desk.click").unwrap().id, desk);
    }

    #[test]
    fn nothing_looks_means_nothing_acts() {
        let ledger = CaptureLedger::new();
        assert_eq!(
            ledger.consume("m", "m.click").unwrap_err(),
            CaptureRefusal::NoLiveCapture { face: "m".into() }
        );
    }

    #[test]
    fn an_expired_capture_authorizes_nothing() {
        let ledger = CaptureLedger::with_ttl(Duration::ZERO);
        let id = ledger.mint("m", "m.observe", json!({}));
        assert_eq!(
            ledger.consume("m", "m.click").unwrap_err(),
            CaptureRefusal::Expired { id }
        );
    }

    #[test]
    fn concurrent_attempts_race_to_exactly_one_winner() {
        use std::sync::Arc;
        let ledger = Arc::new(CaptureLedger::new());
        ledger.mint("m", "m.observe", json!({}));
        let barrier = Arc::new(std::sync::Barrier::new(2));
        let handles: Vec<_> = (0..2)
            .map(|i| {
                let ledger = Arc::clone(&ledger);
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    ledger.consume("m", &format!("m.click{i}"))
                })
            })
            .collect();
        let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        assert_eq!(
            results.iter().filter(|r| r.is_ok()).count(),
            1,
            "exactly one attempt spends the capture: {results:?}"
        );
    }

    #[test]
    fn face_of_takes_the_namespace_root() {
        assert_eq!(face_of("m.observe"), "m");
        assert_eq!(face_of("desk.win_state"), "desk");
    }
}
