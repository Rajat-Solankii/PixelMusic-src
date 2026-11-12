//! Greeting header section: "Good evening, <name>" with time-appropriate greeting.

use chrono::Local;

/// Get time-appropriate greeting based on current hour.
pub fn greeting_for_hour(hour: u32) -> &'static str {
    match hour {
        5..=11 => "Good morning",
        12..=16 => "Good afternoon",
        17..=21 => "Good evening",
        _ => "Good night",
    }
}

/// Get localized greeting based on current time.
pub fn localized_greeting() -> String {
    let hour = Local::now().hour();
    let greeting = greeting_for_hour(hour);
    mf_i18n::t(greeting)
}

/// Build greeting with user name.
pub fn greeting_with_name(user_name: &str) -> String {
    let greeting = localized_greeting();
    if user_name.is_empty() {
        greeting
    } else {
        format!("{}, {}", greeting, user_name)
    }
}

/// Greeting section data.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GreetingSection {
    pub greeting: String,
    pub user_name: String,
}

impl GreetingSection {
    pub fn new(user_name: impl Into<String>) -> Self {
        let name = user_name.into();
        Self {
            greeting: greeting_with_name(&name),
            user_name: name,
        }
    }

    pub fn anonymous() -> Self {
        Self {
            greeting: localized_greeting(),
            user_name: String::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_greeting_for_hour() {
        assert_eq!(greeting_for_hour(6), "Good morning");
        assert_eq!(greeting_for_hour(10), "Good morning");
        assert_eq!(greeting_for_hour(12), "Good afternoon");
        assert_eq!(greeting_for_hour(15), "Good afternoon");
        assert_eq!(greeting_for_hour(18), "Good evening");
        assert_eq!(greeting_for_hour(20), "Good evening");
        assert_eq!(greeting_for_hour(23), "Good night");
        assert_eq!(greeting_for_hour(2), "Good night");
    }

    #[test]
    fn test_greeting_with_name() {
        let greeting = greeting_with_name("Alice");
        assert!(greeting.contains("Alice"));
    }

    #[test]
    fn test_anonymous_greeting() {
        let section = GreetingSection::anonymous();
        assert!(!section.greeting.is_empty());
        assert!(section.user_name.is_empty());
    }
}
