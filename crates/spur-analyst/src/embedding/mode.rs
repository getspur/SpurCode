const ANALYST_EMBED_MODE_ENV: &str = "SPUR_ANALYST_EMBED_MODE";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AnalystEmbedMode {
    Auto,
    InProcess,
    Sidecar,
    Off,
}

impl AnalystEmbedMode {
    pub(crate) fn current() -> Self {
        #[cfg(test)]
        if let Some(mode) =
            ANALYST_EMBED_MODE_OVERRIDE_FOR_TESTS.with(|override_mode| override_mode.get())
        {
            return mode;
        }

        Self::from_env()
    }

    fn from_env() -> Self {
        match std::env::var(ANALYST_EMBED_MODE_ENV) {
            Ok(value) => Self::parse_env_value(&value),
            Err(std::env::VarError::NotPresent) => Self::Auto,
            Err(error) => {
                tracing::warn!(
                    %error,
                    env = ANALYST_EMBED_MODE_ENV,
                    "failed to read analyst embed mode; falling back to auto"
                );
                Self::Auto
            }
        }
    }

    pub(crate) fn parse_env_value(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "auto" => Self::Auto,
            "inprocess" => Self::InProcess,
            "sidecar" => Self::Sidecar,
            "off" => Self::Off,
            _ => {
                tracing::warn!(
                    value,
                    env = ANALYST_EMBED_MODE_ENV,
                    "unknown analyst embed mode; falling back to auto"
                );
                Self::Auto
            }
        }
    }

    #[cfg(all(test, feature = "embed"))]
    pub(crate) fn allows_in_process(self, entrypoint: &'static str) -> bool {
        match self {
            Self::Auto | Self::InProcess => true,
            Self::Off => false,
            Self::Sidecar => {
                tracing::debug!(
                    mode = "sidecar",
                    entrypoint,
                    "analyst embed sidecar mode does not allow in-process model loading"
                );
                false
            }
        }
    }
}

#[cfg(test)]
thread_local! {
    static ANALYST_EMBED_MODE_OVERRIDE_FOR_TESTS: std::cell::Cell<Option<AnalystEmbedMode>> =
        const { std::cell::Cell::new(None) };
}

#[cfg(test)]
pub(crate) struct AnalystEmbedModeOverrideGuard {
    previous: Option<AnalystEmbedMode>,
}

#[cfg(test)]
impl Drop for AnalystEmbedModeOverrideGuard {
    fn drop(&mut self) {
        ANALYST_EMBED_MODE_OVERRIDE_FOR_TESTS.with(|override_mode| {
            override_mode.set(self.previous);
        });
    }
}

#[cfg(test)]
pub(crate) fn set_analyst_embed_mode_for_test(
    mode: AnalystEmbedMode,
) -> AnalystEmbedModeOverrideGuard {
    let previous = ANALYST_EMBED_MODE_OVERRIDE_FOR_TESTS.with(|override_mode| {
        let previous = override_mode.get();
        override_mode.set(Some(mode));
        previous
    });
    AnalystEmbedModeOverrideGuard { previous }
}
