#[cfg(test)]
mod tests {
    use super::super::types::*;

    #[test]
    fn test_card_state_status_text() {
        assert_eq!(CardState::Stopped.status_text(), "Stopped");
        assert_eq!(CardState::Starting.status_text(), "Starting...");
        assert_eq!(CardState::Loading.status_text(), "Loading...");
        assert_eq!(CardState::Ready.status_text(), "Ready");
        assert_eq!(CardState::Error("Err".into()).status_text(), "Err");
    }

    #[test]
    fn test_card_state_is_on() {
        assert!(!CardState::Stopped.is_on());
        assert!(CardState::Starting.is_on());
        assert!(CardState::Loading.is_on());
        assert!(CardState::Ready.is_on());
        assert!(!CardState::Error("Err".into()).is_on());
    }

    #[test]
    fn test_polling_state() {
        assert!(!PollingState::Inactive.is_active());
        assert!(PollingState::Active {
            tokens_used: 100,
            n_ctx: 1000
        }
        .is_active());
        assert!(PollingState::Error.is_active());
    }
}
