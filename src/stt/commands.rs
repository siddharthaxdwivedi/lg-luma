//! Local TV-command matching.
//!
//! Whisper runs in command mode, so most transcripts already look like
//! "open youtube" or "search for ...". Anything that does not resolve to a
//! local action is forwarded to the cloud assistant instead.

use crate::cloud_agent::VoiceAction;

/// Normalise a transcript for matching: lower case, single spaces, and only
/// characters that can appear in an app title or a search phrase.
pub fn normalize(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut space = false;
    for c in text.chars().flat_map(char::to_lowercase) {
        if c.is_alphanumeric() || c == '\'' || c == '-' {
            out.push(c);
            space = false;
        } else if !space && !out.is_empty() {
            out.push(' ');
            space = true;
        }
    }
    while out.ends_with(' ') {
        out.pop();
    }
    out
}

/// Resolve a transcript to a local action, using the installed app list to
/// turn a spoken name into an application id.
pub fn resolve(text: &str, apps: &[(String, String)]) -> Option<VoiceAction> {
    let spoken = normalize(text);
    let (verb, rest) = spoken.split_once(' ')?;
    match verb {
        "open" | "launch" | "start" | "switch" | "go" => {
            let name = match verb {
                "switch" | "go" => rest.strip_prefix("to ").unwrap_or(rest),
                _ => rest,
            };
            find_app(name, apps).map(VoiceAction::Launch)
        }
        "search" | "find" => {
            let query = rest.strip_prefix("for ").unwrap_or(rest).trim();
            if query.is_empty() {
                None
            } else {
                Some(VoiceAction::Search(query.to_string()))
            }
        }
        _ => None,
    }
}

fn find_app(name: &str, apps: &[(String, String)]) -> Option<String> {
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    apps.iter()
        .find(|(title, _)| normalize(title) == name)
        .or_else(|| {
            apps.iter()
                .find(|(title, _)| normalize(title).contains(name))
        })
        .or_else(|| {
            apps.iter()
                .find(|(_, id)| id.to_ascii_lowercase().contains(name))
        })
        .map(|(_, id)| id.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apps() -> Vec<(String, String)> {
        vec![
            ("YouTube".to_string(), "youtube.leanback.v4".to_string()),
            ("Netflix".to_string(), "netflix".to_string()),
            ("Prime Video".to_string(), "amazon".to_string()),
        ]
    }

    #[test]
    fn normalizes_transcripts() {
        assert_eq!(normalize("  Open   YouTube! "), "open youtube");
        assert_eq!(
            normalize("Search for New York's weather?"),
            "search for new york's weather"
        );
        assert_eq!(normalize("..."), "");
    }

    #[test]
    fn resolves_open_and_launch_to_app_ids() {
        assert_eq!(
            resolve("open youtube", &apps()),
            Some(VoiceAction::Launch("youtube.leanback.v4".to_string()))
        );
        assert_eq!(
            resolve("Launch Netflix.", &apps()),
            Some(VoiceAction::Launch("netflix".to_string()))
        );
        assert_eq!(
            resolve("go to prime video", &apps()),
            Some(VoiceAction::Launch("amazon".to_string()))
        );
    }

    #[test]
    fn resolves_search_queries() {
        assert_eq!(
            resolve("search for the weather in Berlin", &apps()),
            Some(VoiceAction::Search("the weather in berlin".to_string()))
        );
        assert_eq!(resolve("search", &apps()), None);
    }

    #[test]
    fn leaves_unmatched_transcripts_for_the_cloud() {
        assert_eq!(resolve("what is the weather", &apps()), None);
        assert_eq!(resolve("open something not installed", &apps()), None);
        assert_eq!(resolve("", &apps()), None);
    }
}
