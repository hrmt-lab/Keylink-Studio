use crate::{ActiveApp, RuleConfig};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayerAction {
    Set(u8),
    Clear,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleMatch<'a> {
    pub rule: &'a RuleConfig,
    pub priority: MatchPriority,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MatchPriority {
    Path = 0,
    Exe = 1,
    Title = 2,
}

pub fn match_action<'a>(
    app: &ActiveApp,
    rules: &'a [RuleConfig],
) -> (LayerAction, Option<RuleMatch<'a>>) {
    if let Some(rule) = first_path_match(app, rules) {
        return (
            LayerAction::Set(rule.layer),
            Some(RuleMatch {
                rule,
                priority: MatchPriority::Path,
            }),
        );
    }
    if let Some(rule) = first_exe_match(app, rules) {
        return (
            LayerAction::Set(rule.layer),
            Some(RuleMatch {
                rule,
                priority: MatchPriority::Exe,
            }),
        );
    }
    if let Some(rule) = first_title_match(app, rules) {
        return (
            LayerAction::Set(rule.layer),
            Some(RuleMatch {
                rule,
                priority: MatchPriority::Title,
            }),
        );
    }
    (LayerAction::Clear, None)
}

fn first_path_match<'a>(app: &ActiveApp, rules: &'a [RuleConfig]) -> Option<&'a RuleConfig> {
    let active_path = app.process_path.as_ref()?.to_string_lossy();
    rules.iter().find(|rule| {
        rule.path
            .as_ref()
            .is_some_and(|path| eq_ignore_case(path, &active_path))
    })
}

fn first_exe_match<'a>(app: &ActiveApp, rules: &'a [RuleConfig]) -> Option<&'a RuleConfig> {
    let exe = app.exe.as_ref()?;
    rules.iter().find(|rule| {
        rule.exe
            .as_ref()
            .is_some_and(|candidate| eq_ignore_case(candidate, exe))
    })
}

fn first_title_match<'a>(app: &ActiveApp, rules: &'a [RuleConfig]) -> Option<&'a RuleConfig> {
    let title = app.title.as_ref()?;
    let title = title.to_lowercase();
    rules.iter().find(|rule| {
        rule.title
            .as_ref()
            .is_some_and(|candidate| title.contains(&candidate.to_lowercase()))
    })
}

fn eq_ignore_case(left: &str, right: &str) -> bool {
    left.eq_ignore_ascii_case(right) || left.to_lowercase() == right.to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn rule(name: &str, layer: u8) -> RuleConfig {
        RuleConfig {
            name: name.to_string(),
            layer,
            path: None,
            exe: None,
            title: None,
        }
    }

    #[test]
    fn path_beats_exe_and_title() {
        let app = ActiveApp {
            process_path: Some(PathBuf::from("C:\\Apps\\Code.exe")),
            exe: Some("Code.exe".to_string()),
            title: Some("GitHub - Project".to_string()),
        };
        let mut path_rule = rule("path", 3);
        path_rule.path = Some("c:\\apps\\code.exe".to_string());
        let mut exe_rule = rule("exe", 1);
        exe_rule.exe = Some("code.exe".to_string());
        let mut title_rule = rule("title", 2);
        title_rule.title = Some("github".to_string());

        let rules = [title_rule, exe_rule, path_rule];
        let (action, matched) = match_action(&app, &rules);

        assert_eq!(action, LayerAction::Set(3));
        assert_eq!(matched.unwrap().priority, MatchPriority::Path);
    }

    #[test]
    fn same_priority_uses_config_order() {
        let app = ActiveApp {
            exe: Some("notepad.exe".to_string()),
            ..ActiveApp::default()
        };
        let mut first = rule("first", 1);
        first.exe = Some("NOTEPAD.EXE".to_string());
        let mut second = rule("second", 2);
        second.exe = Some("notepad.exe".to_string());

        let rules = [first, second];
        let (action, matched) = match_action(&app, &rules);

        assert_eq!(action, LayerAction::Set(1));
        assert_eq!(matched.unwrap().rule.name, "first");
    }

    #[test]
    fn title_uses_case_insensitive_contains() {
        let app = ActiveApp {
            title: Some("Issue - GitHub".to_string()),
            ..ActiveApp::default()
        };
        let mut title = rule("title", 4);
        title.title = Some("github".to_string());

        let (action, _) = match_action(&app, &[title]);

        assert_eq!(action, LayerAction::Set(4));
    }

    #[test]
    fn no_match_clears() {
        let (action, matched) = match_action(&ActiveApp::default(), &[]);

        assert_eq!(action, LayerAction::Clear);
        assert!(matched.is_none());
    }
}
