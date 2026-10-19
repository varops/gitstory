use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

macro_rules! error_codes {
    ($( $(#[$doc:meta])* $variant:ident = $s:literal => $exit:ident, )*) => {
        /// Stable, documented error codes, serialized as `E_*` strings.
        /// Codes never change meaning between versions; new ones may be added.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum ErrorCode {
            $( $(#[$doc])* $variant, )*
        }

        impl ErrorCode {
            pub const ALL: &'static [ErrorCode] = &[$(ErrorCode::$variant,)*];

            pub const fn as_str(self) -> &'static str {
                match self { $( Self::$variant => $s, )* }
            }

            /// Process exit code for this error's class.
            pub const fn exit_code(self) -> ExitCode {
                match self { $( Self::$variant => ExitCode::$exit, )* }
            }

            pub fn parse(s: &str) -> Option<Self> {
                match s { $( $s => Some(Self::$variant), )* _ => None }
            }
        }
    };
}

error_codes! {
    /// Unexpected internal failure.
    Internal = "E_INTERNAL" => Internal,
    /// Invalid arguments or missing input. GitStory never prompts in automation.
    Usage = "E_USAGE" => Usage,
    /// Not inside a Git repository or worktree.
    NotARepo = "E_NOT_A_REPO" => Usage,
    /// The repository has no `.gitstory` file. Run `git story init`.
    NotInitialized = "E_NOT_INITIALIZED" => Usage,
    /// `.gitstory` or user configuration is invalid.
    Config = "E_CONFIG" => Usage,
    /// No story, evidence event or commit matches the reference.
    NotFound = "E_NOT_FOUND" => NotFound,
    /// Command exists in the grammar but is not built yet.
    NotImplemented = "E_NOT_IMPLEMENTED" => NotImplemented,
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Serialize for ErrorCode {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for ErrorCode {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        // Unknown codes from a newer peer degrade to E_INTERNAL.
        Ok(Self::parse(&s).unwrap_or(Self::Internal))
    }
}

/// Exit codes, one per error class. Same classes as ghf.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ExitCode {
    Ok = 0,
    Internal = 1,
    Usage = 2,
    Refused = 3,
    Conflict = 4,
    Remote = 5,
    NotFound = 6,
    NotImplemented = 64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_unique_and_round_trip() {
        let mut seen = std::collections::HashSet::new();
        for c in ErrorCode::ALL {
            assert!(seen.insert(c.as_str()), "duplicate {c}");
            let json = serde_json::to_string(c).unwrap();
            assert_eq!(serde_json::from_str::<ErrorCode>(&json).unwrap(), *c);
        }
    }
}
