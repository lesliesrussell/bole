// bole-aqk
//! Output formatting. Every command renders either human-readable text or,
//! when `--json` is set, a stable JSON value.

/// Controls how command results are rendered.
pub struct Output {
    json: bool,
    quiet: bool,
}

impl Output {
    /// Builds an output renderer from the global flags.
    pub fn new(json: bool, quiet: bool) -> Self {
        Self { json, quiet }
    }

    /// Emits a result. `human` produces the text form; `json` produces the
    /// machine form. The closures are only invoked for the active mode.
    pub fn emit<H, J>(&self, human: H, json: J)
    where
        H: FnOnce() -> String,
        J: FnOnce() -> serde_json::Value,
    {
        if self.quiet {
            return;
        }
        if self.json {
            println!("{}", json());
        } else {
            println!("{}", human());
        }
    }
}
