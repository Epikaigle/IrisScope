//! Current capture session state.

use serde::{Deserialize, Serialize};

/// Eye associated with a capture.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub enum Eye {
    /// No eye has been selected for the current session.
    #[default]
    Unspecified,
    /// Left eye.
    Left,
    /// Right eye.
    Right,
}

impl Eye {
    /// Returns the French label used by the default filename template.
    #[must_use]
    pub const fn filename_label(self) -> &'static str {
        match self {
            Self::Unspecified => "",
            Self::Left => "Gauche",
            Self::Right => "Droit",
        }
    }
}

/// Patient information retained while a sequence of captures is made.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct CaptureSession {
    first_name: String,
    last_name: String,
    eye: Eye,
    #[serde(default)]
    patient_id: Option<u64>,
}

impl CaptureSession {
    /// Creates a session and removes surrounding whitespace from names.
    #[must_use]
    pub fn new(first_name: impl Into<String>, last_name: impl Into<String>, eye: Eye) -> Self {
        Self {
            first_name: first_name.into().trim().to_owned(),
            last_name: last_name.into().trim().to_owned(),
            eye,
            patient_id: None,
        }
    }

    /// Returns the current first name.
    #[must_use]
    pub fn first_name(&self) -> &str {
        &self.first_name
    }

    /// Returns the current last name.
    #[must_use]
    pub fn last_name(&self) -> &str {
        &self.last_name
    }

    /// Returns the selected eye.
    #[must_use]
    pub const fn eye(&self) -> Eye {
        self.eye
    }

    /// Stable dossier identity selected explicitly by the operator.
    #[must_use]
    pub const fn patient_id(&self) -> Option<u64> {
        self.patient_id
    }

    /// Associates this session with an existing dossier. The library validates it
    /// against the stored names before recording a capture.
    pub const fn set_patient_id(&mut self, patient_id: Option<u64>) {
        self.patient_id = patient_id;
    }

    /// Updates the first name.
    pub fn set_first_name(&mut self, first_name: impl Into<String>) {
        let first_name = first_name.into();
        let first_name = first_name.trim();
        if self.first_name != first_name {
            self.patient_id = None;
            first_name.clone_into(&mut self.first_name);
        }
    }

    /// Updates the last name.
    pub fn set_last_name(&mut self, last_name: impl Into<String>) {
        let last_name = last_name.into();
        let last_name = last_name.trim();
        if self.last_name != last_name {
            self.patient_id = None;
            last_name.clone_into(&mut self.last_name);
        }
    }

    /// Updates the selected eye.
    pub const fn set_eye(&mut self, eye: Eye) {
        self.eye = eye;
    }

    /// Ends the current session and removes all patient information.
    pub fn clear(&mut self) {
        self.first_name.clear();
        self.last_name.clear();
        self.eye = Eye::Unspecified;
        self.patient_id = None;
    }

    /// Reports whether the current session contains patient identity information.
    #[must_use]
    pub fn has_identity(&self) -> bool {
        !self.first_name.is_empty() || !self.last_name.is_empty()
    }

    /// Reports whether both required patient names are present.
    #[must_use]
    pub fn has_complete_identity(&self) -> bool {
        !self.first_name.is_empty() && !self.last_name.is_empty()
    }

    /// Reports whether exactly one patient name was entered.
    #[must_use]
    pub fn has_partial_identity(&self) -> bool {
        self.has_identity() && !self.has_complete_identity()
    }
}

#[cfg(test)]
mod tests {
    use super::{CaptureSession, Eye};

    #[test]
    fn session_normalizes_inputs_and_can_be_ended() {
        let mut session = CaptureSession::new("  Jean ", " Dupont\n", Eye::Right);

        assert_eq!(session.first_name(), "Jean");
        assert_eq!(session.last_name(), "Dupont");
        assert_eq!(session.eye(), Eye::Right);
        assert!(session.has_identity());

        session.clear();

        assert_eq!(session, CaptureSession::default());
        assert!(!session.has_identity());
    }

    #[test]
    fn identity_is_complete_only_with_both_names() {
        let anonymous = CaptureSession::new("", "", Eye::Unspecified);
        let partial = CaptureSession::new("Jean", "", Eye::Right);
        let complete = CaptureSession::new("Jean", "Dupont", Eye::Right);
        assert!(!anonymous.has_partial_identity());
        assert!(!anonymous.has_complete_identity());
        assert!(partial.has_partial_identity());
        assert!(!partial.has_complete_identity());
        assert!(!complete.has_partial_identity());
        assert!(complete.has_complete_identity());
    }
}
