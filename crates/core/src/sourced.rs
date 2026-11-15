//! A value together with how it came about, so the display and the logs can tell a measurement from a prediction.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provenance {
    /// Read from what the printer actually did.
    Measured,
    /// Predicted by the slicer, Klipper or an estimator.
    Estimated,
}

/// `origin` names the rule that produced the value, e.g. "encoder"; it is meant for logs and tests.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sourced<T> {
    pub value: T,
    pub provenance: Provenance,
    pub origin: &'static str,
}

impl<T> Sourced<T> {
    pub fn measured(value: T, origin: &'static str) -> Self {
        Self { value, provenance: Provenance::Measured, origin }
    }

    pub fn estimated(value: T, origin: &'static str) -> Self {
        Self { value, provenance: Provenance::Estimated, origin }
    }

    pub fn is_measured(&self) -> bool {
        self.provenance == Provenance::Measured
    }
}

/// The measurement if there is one, else the estimate: a value the printer really produced always beats a prediction.
pub fn prefer_measured<T>(measured: Option<Sourced<T>>, estimated: Option<Sourced<T>>) -> Option<Sourced<T>> {
    measured.or(estimated)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_measurement_beats_an_estimate() {
        let value = prefer_measured(Some(Sourced::measured(1, "m")), Some(Sourced::estimated(2, "e"))).unwrap();

        assert_eq!((value.value, value.provenance, value.origin), (1, Provenance::Measured, "m"));
    }

    #[test]
    fn the_estimate_is_used_without_a_measurement() {
        let value = prefer_measured(None, Some(Sourced::estimated(2, "e"))).unwrap();

        assert_eq!((value.value, value.provenance), (2, Provenance::Estimated));
    }

    #[test]
    fn nothing_stays_nothing() {
        assert_eq!(prefer_measured::<u32>(None, None), None);
    }
}
