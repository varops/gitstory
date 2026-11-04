//! Known gaps: behavior we know is wrong and cannot fix yet, pinned by
//! tests that pass while the gap is open.
//!
//! Fix first. A gap goes here only when the fix has to wait (it needs a
//! decision, a migration or another repository), never as a way to defer a
//! fix that is ready. The rules:
//!
//! - Each test asserts what the code does **today**, and its comment says
//!   what it should do and how to fix it.
//! - A fix makes the test fail. Then flip the assertion to the correct
//!   behavior and move the test next to the feature it covers. A gap closes
//!   only on a real pass. Never delete or weaken a gap test to get green.
//! - Name them `gap_<what is wrong>`.
//!
//! No gaps are open. The last two (unjudged claims kept as supported, and
//! matching test counts overriding the model) were fixed; their tests now
//! live in `validation.rs`.
