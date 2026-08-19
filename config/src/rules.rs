use regex::Regex;

use crownos_config::schema::WindowRule;

/// A rule with its patterns compiled. Rules with a bad regex never become one of
/// these, so a typo in one cannot break the others.
#[derive(Debug)]
pub struct CompiledRule {
    app_id: Option<Regex>,
    title: Option<Regex>,
    rule: WindowRule,
}

impl CompiledRule {
    fn matches(&self, app_id: Option<&str>, title: Option<&str>) -> bool {
        if self.app_id.is_none() && self.title.is_none() {
            return false;
        }
        let app_ok = match &self.app_id {
            Some(re) => app_id.is_some_and(|value| re.is_match(value)),
            None => true,
        };
        let title_ok = match &self.title {
            Some(re) => title.is_some_and(|value| re.is_match(value)),
            None => true,
        };
        app_ok && title_ok
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ResolvedRule {
    pub floating: Option<bool>,
    pub fullscreen: Option<bool>,
    pub maximized: Option<bool>,
    pub workspace: Option<u16>,
    pub output: Option<String>,
    pub focus: Option<bool>,
    pub opacity: Option<f32>,
    pub corner_radius: Option<u16>,
}

#[derive(Debug, Default)]
pub struct WindowRules {
    rules: Vec<CompiledRule>,
}

impl WindowRules {
    pub fn compile(rules: &[WindowRule]) -> Self {
        let mut compiled = Vec::with_capacity(rules.len());

        for (index, rule) in rules.iter().enumerate() {
            let app_id = match compile_pattern(rule.app_id.as_deref(), index, "app_id") {
                Ok(re) => re,
                Err(()) => continue,
            };
            let title = match compile_pattern(rule.title.as_deref(), index, "title") {
                Ok(re) => re,
                Err(()) => continue,
            };
            compiled.push(CompiledRule {
                app_id,
                title,
                rule: rule.clone(),
            });
        }

        Self { rules: compiled }
    }

    pub fn resolve(&self, app_id: Option<&str>, title: Option<&str>) -> ResolvedRule {
        let mut out = ResolvedRule::default();

        for compiled in self.rules.iter().filter(|r| r.matches(app_id, title)) {
            let rule = &compiled.rule;
            out.floating = rule.floating.or(out.floating);
            out.fullscreen = rule.fullscreen.or(out.fullscreen);
            out.maximized = rule.maximized.or(out.maximized);
            out.workspace = rule.workspace.or(out.workspace);
            out.output = rule.output.clone().or(out.output);
            out.focus = rule.focus.or(out.focus);
            out.opacity = rule.opacity.or(out.opacity);
            out.corner_radius = rule.corner_radius.or(out.corner_radius);
        }

        out
    }
}

fn compile_pattern(pattern: Option<&str>, index: usize, field: &str) -> Result<Option<Regex>, ()> {
    match pattern {
        None => Ok(None),
        Some(pattern) => match Regex::new(pattern) {
            Ok(re) => Ok(Some(re)),
            Err(err) => {
                tracing::warn!(
                    %err, rule = index, field, pattern,
                    "ignoring window rule with an invalid regex"
                );
                Err(())
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(app_id: Option<&str>, title: Option<&str>) -> WindowRule {
        WindowRule {
            app_id: app_id.map(str::to_owned),
            title: title.map(str::to_owned),
            floating: Some(true),
            ..Default::default()
        }
    }

    #[test]
    fn matches_on_app_id() {
        let rules = WindowRules::compile(&[rule(Some("^blender$"), None)]);
        assert_eq!(
            rules.resolve(Some("blender"), None).floating,
            Some(true),
            "an exact app_id match should apply"
        );
        assert_eq!(rules.resolve(Some("foot"), None).floating, None);
    }

    #[test]
    fn both_patterns_must_match() {
        let rules = WindowRules::compile(&[rule(Some("blender"), Some("Preferences"))]);
        assert_eq!(
            rules
                .resolve(Some("blender"), Some("Blender Preferences"))
                .floating,
            Some(true)
        );
        assert_eq!(
            rules
                .resolve(Some("blender"), Some("untitled.blend"))
                .floating,
            None,
            "the title pattern must also match"
        );
    }

    #[test]
    fn empty_rule_matches_nothing() {
        let rules = WindowRules::compile(&[rule(None, None)]);
        assert_eq!(
            rules.resolve(Some("anything"), Some("anything")).floating,
            None,
            "a rule with no patterns must not float the whole desktop"
        );
    }

    #[test]
    fn invalid_regex_is_skipped_not_fatal() {
        let rules = WindowRules::compile(&[rule(Some("("), None), rule(Some("foot"), None)]);
        assert_eq!(
            rules.resolve(Some("foot"), None).floating,
            Some(true),
            "one bad regex must not discard the rules around it"
        );
    }

    #[test]
    fn later_rules_win_per_field() {
        let broad = WindowRule {
            app_id: Some("foot".into()),
            floating: Some(true),
            opacity: Some(0.9),
            ..Default::default()
        };
        let narrow = WindowRule {
            app_id: Some("foot".into()),
            floating: Some(false),
            ..Default::default()
        };
        let rules = WindowRules::compile(&[broad, narrow]);
        let resolved = rules.resolve(Some("foot"), None);
        assert_eq!(resolved.floating, Some(false), "the later rule wins");
        assert_eq!(
            resolved.opacity,
            Some(0.9),
            "fields it does not set are kept"
        );
    }
}
