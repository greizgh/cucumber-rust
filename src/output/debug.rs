use std;
use std::path::Path;

use gherkin;

use crate::OutputVisitor;
use crate::TestResult;

pub struct DebugOutput;

impl OutputVisitor for DebugOutput {
    fn new() -> Self
    where
        Self: Sized,
    {
        DebugOutput
    }

    fn visit_start(&self) {
        println!("visit_start");
    }

    fn visit_feature(&self, feature: &gherkin::Feature, path: &Path) {
        println!("visit_feature {} {}", feature.name, path.display());
    }

    fn visit_feature_end(&self, feature: &gherkin::Feature) {
        println!("visit_feature_end {}", feature.name);
    }

    fn visit_feature_error(&self, path: &Path, error: &gherkin::TryFromPathError) {
        println!("visit_feature_error {} {:?}", path.display(), error);
    }

    fn visit_rule(&self, rule: &gherkin::Rule) {
        println!("visit_rule {}", rule.name);
    }

    fn visit_rule_end(&self, rule: &gherkin::Rule) {
        println!("visit_rule_end {}", rule.name);
    }

    fn visit_scenario(&self, _rule: Option<&gherkin::Rule>, scenario: &crate::Scenario) {
        println!("visit_scenario {}", scenario.name);
    }

    fn visit_scenario_end(&self, _rule: Option<&gherkin::Rule>, scenario: &crate::Scenario) {
        println!("visit_scenario_end {}", scenario.name);
    }

    fn visit_scenario_skipped(
        &self,
        _rule: Option<&gherkin::Rule>,
        scenario: &crate::Scenario,
    ) {
        println!("visit_scenario_skipped {}", scenario.name);
    }

    fn visit_step(
        &self,
        _rule: Option<&gherkin::Rule>,
        _scenario: &crate::Scenario,
        step: &crate::Step,
    ) {
        println!("visit_step {} {}", step.raw_type, step.value);
    }

    fn visit_step_resolved<W: crate::World>(
        &self,
        _step: &crate::Step,
        test: &crate::steps::TestPayload<W>,
    ) {
        println!("visit_step_resolved {:?}", &test.meta);
    }

    fn visit_step_result(
        &self,
        _rule: Option<&gherkin::Rule>,
        _scenario: &crate::Scenario,
        step: &crate::Step,
        result: &TestResult,
    ) {
        println!(
            "visit_step_result {} {} - {:?}",
            step.raw_type, step.value, result
        );
    }

    fn visit_finish(&self) {
        println!("visit_finish");
    }
}
