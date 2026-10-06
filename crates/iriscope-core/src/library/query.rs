//! Validated library filters; candidate hints are checked again on resolved pages.
use super::LibraryScanCandidate;
use crate::session::Eye;
use chrono::NaiveDate;

/// Search criteria independent of the media-type filter and selected session.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct LibraryQuery {
    /// Exact stable dossier number, never a name-based association.
    pub patient_id: Option<u64>,
    /// Inclusive start date in ISO format.
    pub date_from: String,
    /// Inclusive end date in ISO format.
    pub date_to: String,
    /// Eye to include, or all eyes.
    pub eye: Option<Eye>,
    /// Reverse the default newest-first order.
    pub oldest_first: bool,
}
impl LibraryQuery {
    /// Validates user input without replacing a previously applied query on error.
    /// # Errors
    /// Returns a French explanation for invalid IDs, dates, or ranges.
    pub fn parse(dossier: &str, from: &str, to: &str, eye: i32, sort: i32) -> Result<Self, String> {
        let dossier = dossier.trim();
        let patient_id = if dossier.is_empty() {
            None
        } else {
            let digits = dossier
                .strip_prefix("D-")
                .or_else(|| dossier.strip_prefix("d-"))
                .unwrap_or(dossier);
            if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
                return Err("Numéro de dossier invalide. Exemple : D-000123.".into());
            }
            Some(
                digits
                    .parse::<u64>()
                    .ok()
                    .filter(|id| *id > 0)
                    .ok_or("Numéro de dossier invalide. Exemple : D-000123.")?,
            )
        };
        let date = |value: &str| -> Result<String, String> {
            let value = value.trim();
            if value.is_empty() {
                return Ok(String::new());
            }
            if value.len() == 10 && value.as_bytes()[2] == b'/' && value.as_bytes()[5] == b'/' {
                return NaiveDate::parse_from_str(value, "%d/%m/%Y")
                    .map(|d| d.format("%Y-%m-%d").to_string())
                    .map_err(|_| "Date invalide. Utilisez JJ/MM/AAAA.".to_owned());
            }
            if !value.is_empty()
                && (value.len() != 10
                    || !value.bytes().enumerate().all(|(i, byte)| {
                        if i == 4 || i == 7 {
                            byte == b'-'
                        } else {
                            byte.is_ascii_digit()
                        }
                    })
                    || NaiveDate::parse_from_str(value, "%Y-%m-%d").is_err())
            {
                return Err("Date invalide. Utilisez JJ/MM/AAAA, par exemple 05/10/2026.".into());
            }
            Ok(value.to_owned())
        };
        let date_from = date(from)?;
        let date_to = date(to)?;
        if !date_from.is_empty() && !date_to.is_empty() && date_from > date_to {
            return Err("La date de début doit précéder la date de fin.".into());
        }
        let eye = match eye {
            0 => None,
            1 => Some(Eye::Left),
            2 => Some(Eye::Right),
            3 => Some(Eye::Unspecified),
            _ => return Err("Filtre d’œil invalide.".into()),
        };
        if !(0..=1).contains(&sort) {
            return Err("Tri invalide.".into());
        }
        Ok(Self {
            patient_id,
            date_from,
            date_to,
            eye,
            oldest_first: sort == 1,
        })
    }
    /// Tests anonymous metadata; callers must verify associations before display.
    #[must_use]
    pub fn matches(&self, candidate: &LibraryScanCandidate) -> bool {
        self.patient_id
            .is_none_or(|id| candidate.patient_id_hint == Some(id))
            && self.eye.is_none_or(|eye| candidate.eye_hint == eye)
            && (self.date_from.is_empty() || candidate.date_str >= self.date_from)
            && (self.date_to.is_empty()
                || (!candidate.date_str.is_empty() && candidate.date_str <= self.date_to))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn query_rejects_invalid_ids_dates_and_inverted_ranges() {
        for id in ["D-0", "abc", "-4"] {
            assert!(LibraryQuery::parse(id, "", "", 0, 0).is_err());
        }
        for date in ["2026-02-30", "2026-1-01", "yesterday"] {
            assert!(LibraryQuery::parse("", date, "", 0, 0).is_err());
        }
        assert!(LibraryQuery::parse("", "2026-10-05", "2026-01-01", 0, 0).is_err());
        let query = LibraryQuery::parse("d-000042", "2026-01-01", "2026-10-05", 2, 1).unwrap();
        assert_eq!(query.patient_id, Some(42));
        assert_eq!(query.eye, Some(Eye::Right));
        assert!(query.oldest_first);
        let french = LibraryQuery::parse("D-42", "01/01/2026", "05/10/2026", 2, 1).unwrap();
        assert_eq!(french, query);
        assert!(LibraryQuery::parse("", "30/02/2026", "", 0, 0).is_err());
    }
}
