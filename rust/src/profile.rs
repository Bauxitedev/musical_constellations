use std::{fmt, time::Duration};

// Note - the local variables here don't need unique variable names, as macro_rules is hygienic
#[macro_export]
macro_rules! profile {
    ($label:expr, $expr:expr) => {{
        let start = std::time::Instant::now();
        let result = $expr;
        let duration = start.elapsed();
        tracing::debug!(func = %$label, time = %$crate::profile::Ms(duration), "profile!");

        result
    }};

    ($expr:expr) => {{
        let label = stringify!($expr);
        $crate::profile!(label, $expr)
    }};
}

/// Little wrapper type so we can format duration always as milliseconds, and without " around it (which happens if you simply convert it to a string)
pub struct Ms(pub Duration);

impl fmt::Display for Ms {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let ms = self.0.as_millis_f64();
        write!(f, "{ms:.3}ms")
    }
}
