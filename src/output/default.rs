use std::collections::HashMap;
use std::default::Default;
use std::env;
use std::io::Write;
use std::path::Path;

use gherkin;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use pathdiff::diff_paths;
use std::sync::atomic::AtomicU32;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, RwLock};
use termcolor::{BufferWriter, Color, ColorChoice, ColorSpec, WriteColor};
use textwrap;

use crate::OutputVisitor;
use crate::TestResult;

enum ScenarioResult {
    Pass,
    Fail,
    Skip,
}

pub struct DefaultOutput {
    stdout: Arc<Mutex<BufferWriter>>,
    cur_feature: Arc<RwLock<String>>,
    feature_count: AtomicU32,
    feature_error_count: AtomicU32,
    rule_count: AtomicU32,
    scenarios: Arc<RwLock<HashMap<gherkin::Scenario, ScenarioResult>>>,
    step_count: AtomicU32,
    skipped_count: AtomicU32,
    fail_count: AtomicU32,
    multi: Arc<Mutex<MultiProgress>>,
    rules_progress: Arc<RwLock<HashMap<gherkin::Rule, (ProgressBar, termcolor::Buffer)>>>,
    progress: Arc<RwLock<HashMap<gherkin::Scenario, (ProgressBar, termcolor::Buffer)>>>,
    progress_handle: Arc<Mutex<Option<std::thread::JoinHandle<()>>>>,
}

impl Default for DefaultOutput {
    fn default() -> DefaultOutput {
        DefaultOutput {
            stdout: Arc::new(Mutex::new(BufferWriter::stdout(ColorChoice::Always))),
            cur_feature: Arc::new(RwLock::new("".to_string())),
            feature_count: AtomicU32::new(0),
            feature_error_count: AtomicU32::new(0),
            rule_count: AtomicU32::new(0),
            scenarios: Arc::new(RwLock::new(HashMap::new())),
            step_count: AtomicU32::new(0),
            skipped_count: AtomicU32::new(0),
            fail_count: AtomicU32::new(0),
            multi: Arc::new(Mutex::new(MultiProgress::new())),
            rules_progress: Arc::new(RwLock::new(HashMap::new())),
            progress: Arc::new(RwLock::new(HashMap::new())),
            progress_handle: Arc::new(Mutex::new(None)),
        }
    }
}

fn sty_rule() -> ProgressStyle {
    ProgressStyle::default_bar().template("{spinner} Rule: {prefix}")
}

fn sty_scenario(indent: usize) -> ProgressStyle {
    let mut m = format!("{:indent$}", "", indent = indent);
    m.push_str("{spinner} Scenario: {prefix} :: {msg}");
    ProgressStyle::default_bar().template(&m)
}

fn sty_scenario_finish(indent: usize, icon: &str) -> ProgressStyle {
    let mut m = format!("{:indent$}", "", indent = indent);
    m.push_str(icon);
    m.push_str(" Scenario: {prefix}");
    ProgressStyle::default_bar().template(&m)
}

fn wrap_with_comment(s: &str, c: &str, indent: &str) -> String {
    let tw = textwrap::termwidth();
    let w = tw - indent.chars().count();
    let mut cs: Vec<String> = textwrap::wrap_iter(s, w)
        .map(|x| format!("{}{}", indent, &x.trim()))
        .collect();
    // Fit the comment onto the last line
    let comment_space = tw.saturating_sub(c.chars().count()).saturating_sub(2);
    let last_count = cs.last().unwrap().chars().count();
    if last_count > comment_space {
        cs.push(format!("{: <1$}", "", comment_space))
    } else {
        cs.last_mut()
            .unwrap()
            .push_str(&format!("{: <1$}", "", comment_space - last_count));
    }
    cs.join("\n")
}

impl DefaultOutput {
    fn set_color(&self, stdout: &mut termcolor::Buffer, c: Color, b: bool) {
        stdout
            .set_color(ColorSpec::new().set_fg(Some(c)).set_bold(b))
            .unwrap();
    }

    fn write(&self, stdout: &mut termcolor::Buffer, s: &str, c: Color, bold: bool) {
        stdout
            .set_color(ColorSpec::new().set_fg(Some(c)).set_bold(bold))
            .unwrap();
        write!(stdout, "{}", s).unwrap();
        stdout
            .set_color(ColorSpec::new().set_fg(None).set_bold(false))
            .unwrap();
    }

    fn writeln(&self, stdout: &mut termcolor::Buffer, s: &str, c: Color, bold: bool) {
        stdout
            .set_color(ColorSpec::new().set_fg(Some(c)).set_bold(bold))
            .unwrap();
        writeln!(stdout, "{}", s).unwrap();
        stdout
            .set_color(ColorSpec::new().set_fg(None).set_bold(false))
            .unwrap();
    }

    fn writeln_cmt(
        &self,
        stdout: &mut termcolor::Buffer,
        s: &str,
        cmt: &str,
        indent: &str,
        c: Color,
        bold: bool,
    ) {
        stdout
            .set_color(ColorSpec::new().set_fg(Some(c)).set_bold(bold))
            .unwrap();
        write!(stdout, "{}", wrap_with_comment(s, cmt, indent)).unwrap();
        stdout
            .set_color(ColorSpec::new().set_fg(Some(Color::White)).set_bold(false))
            .unwrap();
        writeln!(stdout, " {}", cmt).unwrap();
        stdout.set_color(ColorSpec::new().set_fg(None)).unwrap();
    }

    fn println(&self, stdout: &mut termcolor::Buffer, s: &str) {
        writeln!(stdout, "{}", s).unwrap();
    }

    fn red(&self, stdout: &mut termcolor::Buffer, s: &str) {
        self.writeln(stdout, s, Color::Red, false);
    }

    fn bold_white(&self, stdout: &mut termcolor::Buffer, s: &str) {
        self.writeln(stdout, s, Color::Green, true);
    }

    fn bold_white_comment(&self, stdout: &mut termcolor::Buffer, s: &str, c: &str, indent: &str) {
        self.writeln_cmt(stdout, s, c, indent, Color::White, true);
    }

    fn relpath(&self, target: &Path) -> std::path::PathBuf {
        let target = target.canonicalize().expect("invalid target path");
        diff_paths(
            &target,
            &env::current_dir().expect("invalid current directory"),
        )
        .expect("invalid target path")
    }

    fn print_step_extras(&self, stdout: &mut termcolor::Buffer, step: &gherkin::Step) {
        let indent = "      ";
        if let Some(ref table) = &step.table {
            // Find largest sized item per column
            let mut max_size: Vec<usize> = (&table.header).iter().map(|h| h.len()).collect();

            for row in &table.rows {
                for (n, field) in row.iter().enumerate() {
                    if field.len() > max_size[n] {
                        max_size[n] = field.len();
                    }
                }
            }

            // If number print in a number way
            let formatted_header_fields: Vec<String> = (&table.header)
                .iter()
                .enumerate()
                .map(|(n, field)| format!(" {: <1$} ", field, max_size[n]))
                .collect();

            let formatted_row_fields: Vec<Vec<String>> = (&table.rows)
                .iter()
                .map(|row| {
                    row.iter()
                        .enumerate()
                        .map(|(n, field)| {
                            if field.parse::<f64>().is_ok() {
                                format!(" {: >1$} ", field, max_size[n])
                            } else {
                                format!(" {: <1$} ", field, max_size[n])
                            }
                        })
                        .collect()
                })
                .collect();

            write!(stdout, "{}", indent).unwrap();
            let border_color = Color::Magenta;
            self.write(stdout, "|", border_color, true);
            for field in formatted_header_fields {
                self.write(stdout, &field, Color::White, true);
                self.write(stdout, "|", border_color, true);
            }
            self.println(stdout, "");

            for row in formatted_row_fields {
                write!(stdout, "{}", indent).unwrap();
                self.write(stdout, "|", border_color, false);
                for field in row {
                    write!(stdout, "{}", field).unwrap();
                    self.write(stdout, "|", border_color, false);
                }
                self.println(stdout, "");
            }
        };

        if let Some(ref docstring) = &step.docstring {
            self.writeln(stdout, &format!("{}\"\"\"", indent), Color::Magenta, true);
            writeln!(stdout, "{}", textwrap::indent(docstring, indent).trim_end()).unwrap();
            self.writeln(stdout, &format!("{}\"\"\"", indent), Color::Magenta, true);
        }
    }

    fn print_finish(&self) -> Result<(), std::io::Error> {
        let stdout_writer = self.stdout.lock().unwrap();
        let mut stdout = stdout_writer.buffer();
        self.set_color(&mut stdout, Color::White, true);

        // Do feature count
        write!(
            &mut stdout,
            "{} features",
            &self.feature_count.load(Ordering::SeqCst)
        )?;
        let feature_error_count = self.feature_error_count.load(Ordering::SeqCst);
        if feature_error_count > 0 {
            write!(&mut stdout, " (")?;
            self.set_color(&mut stdout, Color::Red, true);
            write!(&mut stdout, "{} errored", feature_error_count)?;
            self.set_color(&mut stdout, Color::White, true);
            write!(&mut stdout, ")")?;
        }

        // Do rule count
        let rule_count = self.rule_count.load(Ordering::SeqCst);
        if rule_count > 0 {
            write!(&mut stdout, ", {} rules", rule_count)?;
        }

        self.println(&mut stdout, "");

        // Do scenario count
        let scenario_passed_count = self
            .scenarios
            .read()
            .unwrap()
            .values()
            .filter(|v| match v {
                ScenarioResult::Pass => true,
                _ => false,
            })
            .count();
        let scenario_fail_count = self
            .scenarios
            .read()
            .unwrap()
            .values()
            .filter(|v| match v {
                ScenarioResult::Fail => true,
                _ => false,
            })
            .count();
        let scenario_skipped_count = self
            .scenarios
            .read()
            .unwrap()
            .values()
            .filter(|v| match v {
                ScenarioResult::Skip => true,
                _ => false,
            })
            .count();

        write!(
            &mut stdout,
            "{} scenarios (",
            &self.scenarios.read().unwrap().len()
        )?;

        if scenario_fail_count > 0 {
            self.set_color(&mut stdout, Color::Red, true);
            write!(&mut stdout, "{} failed", scenario_fail_count)?;
            self.set_color(&mut stdout, Color::White, true);
        }

        if scenario_skipped_count > 0 {
            if scenario_fail_count > 0 {
                write!(&mut stdout, ", ")?;
            }
            self.set_color(&mut stdout, Color::Cyan, true);
            write!(&mut stdout, "{} skipped", scenario_skipped_count)?;
            self.set_color(&mut stdout, Color::White, true);
        }

        if scenario_fail_count > 0 || scenario_skipped_count > 0 {
            write!(&mut stdout, ", ")?;
        }

        self.set_color(&mut stdout, Color::Green, true);
        write!(&mut stdout, "{} passed", scenario_passed_count)?;
        self.set_color(&mut stdout, Color::White, true);

        write!(&mut stdout, ")")?;

        self.println(&mut stdout, "");

        let step_count = self.step_count.load(Ordering::SeqCst);
        let skipped_count = self.skipped_count.load(Ordering::SeqCst);
        let fail_count = self.fail_count.load(Ordering::SeqCst);

        // Do steps
        let passed_count = step_count - skipped_count - fail_count;

        write!(&mut stdout, "{} steps (", step_count)?;

        if fail_count > 0 {
            self.set_color(&mut stdout, Color::Red, true);
            write!(&mut stdout, "{} failed", fail_count)?;
            self.set_color(&mut stdout, Color::White, true);
        }

        if skipped_count > 0 {
            if fail_count > 0 {
                write!(&mut stdout, ", ")?;
            }
            self.set_color(&mut stdout, Color::Cyan, true);
            write!(&mut stdout, "{} skipped", skipped_count)?;
            self.set_color(&mut stdout, Color::White, true);
        }

        if fail_count > 0 || skipped_count > 0 {
            write!(&mut stdout, ", ")?;
        }

        self.set_color(&mut stdout, Color::Green, true);
        write!(&mut stdout, "{} passed", passed_count)?;
        self.set_color(&mut stdout, Color::White, true);
        write!(&mut stdout, ")")?;
        self.println(&mut stdout, "");

        stdout.set_color(ColorSpec::new().set_fg(None).set_bold(false))?;
        self.println(&mut stdout, "");

        stdout_writer.print(&stdout).unwrap();

        Ok(())
    }
}

#[inline]
fn error_position(error: &gherkin::Error) -> (usize, usize) {
    use gherkin::pest::error::LineColLocation;

    match error.line_col {
        LineColLocation::Pos(v) => v,
        LineColLocation::Span(v, _) => v,
    }
}

impl OutputVisitor for DefaultOutput {
    fn new() -> Self {
        Default::default()
    }

    fn visit_start(&self) {
        let stdout = self.stdout.lock().unwrap();
        let mut buffer = stdout.buffer();
        self.bold_white(
            &mut buffer,
            &format!("[Cucumber v{}]\n", env!("CARGO_PKG_VERSION")),
        );
        stdout.print(&buffer).unwrap();
    }

    fn visit_feature(&self, feature: &gherkin::Feature, path: &Path) {
        let cur_feature = self.relpath(&path).to_string_lossy().to_string();

        let msg = &format!("Feature: {}", &feature.name);
        let cmt = &format!(
            "{}:{}:{}",
            &cur_feature, feature.position.0, feature.position.1
        );

        let stdout = self.stdout.lock().unwrap();
        let mut buffer = stdout.buffer();
        self.bold_white_comment(&mut buffer, msg, cmt, "");
        self.println(&mut buffer, "");
        stdout.print(&buffer).unwrap();

        {
            *self.cur_feature.write().unwrap() = cur_feature;
        }

        self.feature_count.fetch_add(1, Ordering::SeqCst);

        let mut multi = self.multi.lock().unwrap();
        *multi = MultiProgress::new();
        multi.set_move_cursor(true);
        let mut rules_pb = HashMap::new();
        let mut scenarios_pb = HashMap::new();

        for rule in feature.rules.iter() {
            let pb = multi.add(ProgressBar::new(rule.scenarios.len() as u64));
            pb.set_style(sty_rule());
            pb.set_prefix(&rule.name);
            pb.set_message("starting…");
            pb.enable_steady_tick(50);
            rules_pb.insert(rule.clone(), (pb, stdout.buffer()));

            for scenario in rule.scenarios.iter() {
                let pb = multi.add(ProgressBar::new(scenario.steps.len() as u64));
                pb.set_style(sty_scenario(1));
                pb.set_prefix(&scenario.name);
                pb.enable_steady_tick(50);
                pb.set_message("starting…");
                scenarios_pb.insert(scenario.clone(), (pb, stdout.buffer()));
            }
        }

        for scenario in feature.scenarios.iter() {
            let pb = multi.add(ProgressBar::new(scenario.steps.len() as u64));
            pb.set_style(sty_scenario(0));
            pb.set_prefix(&scenario.name);
            pb.enable_steady_tick(50);
            pb.set_message("starting…");
            scenarios_pb.insert(scenario.clone(), (pb, stdout.buffer()));
        }

        *self.rules_progress.write().unwrap() = rules_pb;
        *self.progress.write().unwrap() = scenarios_pb;

        let multi = Arc::clone(&self.multi);
        *self.progress_handle.lock().unwrap() = Some(std::thread::spawn(move || {
            multi.lock().unwrap().join_and_clear().unwrap();
        }));
    }

    fn visit_feature_end(&self, feature: &gherkin::Feature) {
        if let Some(handle) = self.progress_handle.lock().unwrap().take() {
            handle.join().unwrap();
        }

        let stdout = self.stdout.lock().unwrap();
        let mut rules_progress = self.rules_progress.write().unwrap();
        let mut progress = self.progress.write().unwrap();

        for rule in feature.rules.iter() {
            let (_, buffer) = rules_progress.remove(rule).unwrap();
            stdout.print(&buffer).unwrap();

            for scenario in rule.scenarios.iter() {
                let (_, buffer) = progress.remove(scenario).unwrap();
                stdout.print(&buffer).unwrap();
            }
        }

        for scenario in feature.scenarios.iter() {
            let (_, buffer) = progress.remove(scenario).unwrap();
            stdout.print(&buffer).unwrap();
        }

        rules_progress.clear();
        progress.clear();
    }

    fn visit_feature_error(&self, path: &Path, error: &gherkin::TryFromPathError) {
        let relpath = self.relpath(&path).to_string_lossy().to_string();
        let stdout_writer = self.stdout.lock().unwrap();
        let mut stdout = stdout_writer.buffer();

        match error {
            gherkin::TryFromPathError::Parsing(error) => {
                let position = error_position(error);
                let loc = &format!("{}:{}:{}", &relpath, position.0, position.1);

                self.writeln_cmt(
                    &mut stdout,
                    &format!(
                        "{:—<1$}",
                        "! Parsing feature failed: ",
                        textwrap::termwidth() - loc.chars().count() - 7
                    ),
                    &loc,
                    "———— ",
                    Color::Red,
                    true,
                );

                self.red(
                    &mut stdout,
                    &textwrap::indent(
                        &textwrap::fill(&format!("{}", error), textwrap::termwidth() - 4),
                        "  ",
                    )
                    .trim_end(),
                );

                self.writeln(
                    &mut stdout,
                    &format!("{:—<1$}\n", "", textwrap::termwidth()),
                    Color::Red,
                    true,
                );
            }
            gherkin::TryFromPathError::Io(error) => {
                self.writeln_cmt(
                    &mut stdout,
                    &format!(
                        "{:—<1$}",
                        "! Parsing feature failed: ",
                        textwrap::termwidth() - relpath.chars().count() - 7
                    ),
                    &relpath,
                    "———— ",
                    Color::Red,
                    true,
                );

                self.red(
                    &mut stdout,
                    &textwrap::indent(
                        &textwrap::fill(&format!("{}", error), textwrap::termwidth() - 4),
                        "  ",
                    )
                    .trim_end(),
                );

                self.writeln(
                    &mut stdout,
                    &format!("{:—<1$}\n", "", textwrap::termwidth()),
                    Color::Red,
                    true,
                );
            }
        }

        stdout_writer.print(&stdout).unwrap();

        self.feature_error_count.fetch_add(1, Ordering::SeqCst);
    }

    fn visit_rule(&self, rule: &gherkin::Rule) {
        let mut guard = self.rules_progress.write().unwrap();
        let mut buffer = &mut guard.get_mut(rule).unwrap().1;

        let cmt = &format!(
            "{}:{}:{}",
            &self.cur_feature.read().unwrap(),
            rule.position.0,
            rule.position.1
        );
        self.bold_white_comment(&mut buffer, &format!("Rule: {}\n", &rule.name), cmt, " ");
    }

    fn visit_rule_end(&self, rule: &gherkin::Rule) {
        self.rule_count.fetch_add(1, Ordering::SeqCst);

        self.rules_progress.write().unwrap()[rule].0.finish();
    }

    fn visit_scenario(&self, rule: Option<&gherkin::Rule>, scenario: &gherkin::Scenario) {
        let mut guard = self.progress.write().unwrap();
        let mut buffer = match guard.get_mut(scenario) {
            Some(v) => &mut v.1,
            None => {
                return;
            }
        };
        let cmt = &format!(
            "{}:{}:{}",
            &self.cur_feature.read().unwrap(),
            scenario.position.0,
            scenario.position.1
        );
        let indent = if rule.is_some() { "  " } else { " " };
        self.bold_white_comment(
            &mut buffer,
            &format!("Scenario: {}", &scenario.name),
            cmt,
            indent,
        );

        if let Some(rule) = rule {
            self.rules_progress.write().unwrap()[rule].0.inc(1);
        }
    }

    fn visit_scenario_skipped(&self, _rule: Option<&gherkin::Rule>, _scenario: &gherkin::Scenario) {
        // if !self.scenarios.read().unwrap().contains_key(scenario) {
        //     self.scenarios
        //         .write()
        //         .unwrap()
        //         .insert(scenario.clone(), ScenarioResult::Skip);

        //     let pb = &self.progress.write().unwrap()[scenario].0;
        // }
    }

    fn visit_scenario_end(&self, rule: Option<&gherkin::Rule>, scenario: &gherkin::Scenario) {
        if !self.scenarios.read().unwrap().contains_key(scenario) {
            self.scenarios
                .write()
                .unwrap()
                .insert(scenario.clone(), ScenarioResult::Pass);
        }

        let mut guard = self.progress.write().unwrap();
        let mut buffer = match guard.get_mut(scenario) {
            Some(v) => &mut v.1,
            None => return,
        };
        self.println(&mut buffer, "");
        let pb = &guard[scenario].0;
        let status = &self.scenarios.read().unwrap()[scenario];
        let (icon, msg) = match status {
            ScenarioResult::Pass => ("✔", "passed"),
            ScenarioResult::Fail => ("✘", "failed"),
            _ => ("⚡", "skipped"),
        };
        let indent = if rule.is_some() { 1 } else { 0 };
        pb.set_style(sty_scenario_finish(indent, icon));
        pb.finish_with_message(msg);
    }

    fn visit_step(
        &self,
        _rule: Option<&gherkin::Rule>,
        scenario: &gherkin::Scenario,
        step: &gherkin::Step,
    ) {
        self.step_count.fetch_add(1, Ordering::SeqCst);
        let guard = self.progress.write().unwrap();
        let pb = match guard.get(scenario) {
            Some(v) => &v.0,
            None => return,
        };
        pb.set_message(&format!("Step: {}", &step.to_string()));
        pb.inc(1);
    }

    fn visit_step_resolved<W: crate::World>(
        &self,
        _step: &crate::Step,
        _test: &crate::steps::TestPayload<W>,
    ) {
    }

    fn visit_step_result(
        &self,
        rule: Option<&gherkin::Rule>,
        scenario: &gherkin::Scenario,
        step: &gherkin::Step,
        result: &TestResult,
    ) {
        let cmt = &format!(
            "{}:{}:{}",
            &self.cur_feature.read().unwrap(),
            step.position.0,
            step.position.1
        );
        let msg = &step.to_string();
        let indent = if rule.is_some() { "   " } else { "  " };

        let mut guard = self.progress.write().unwrap();
        let item = match guard.get_mut(scenario) {
            Some(v) => v,
            None => return,
        };
        let pb = &item.0;
        let mut buffer = &mut item.1;

        match result {
            TestResult::Pass => {
                self.writeln_cmt(
                    &mut buffer,
                    &format!("✔ {}", msg),
                    cmt,
                    indent,
                    Color::Green,
                    false,
                );
                self.print_step_extras(&mut buffer, step);
            }
            TestResult::Fail(panic_info, captured_stdout, captured_stderr) => {
                pb.finish_and_clear();
                self.writeln_cmt(
                    &mut buffer,
                    &format!("✘ {}", msg),
                    cmt,
                    indent,
                    Color::Red,
                    false,
                );
                self.print_step_extras(&mut buffer, step);
                self.writeln_cmt(
                    &mut buffer,
                    &format!(
                        "{:—<1$}",
                        "! Step failed: ",
                        textwrap::termwidth()
                            .saturating_sub(panic_info.location.chars().count())
                            .saturating_sub(7),
                    ),
                    &panic_info.location,
                    "———— ",
                    Color::Red,
                    true,
                );
                self.red(
                    &mut buffer,
                    &textwrap::indent(
                        &textwrap::fill(&panic_info.payload, textwrap::termwidth() - 4),
                        "  ",
                    )
                    .trim_end(),
                );

                if !captured_stdout.is_empty() {
                    self.writeln(
                        &mut buffer,
                        &format!("{:—<1$}", "———— Captured stdout: ", textwrap::termwidth()),
                        Color::Red,
                        true,
                    );
                    self.red(
                        &mut buffer,
                        &textwrap::indent(
                            &textwrap::fill(
                                &String::from_utf8_lossy(captured_stderr),
                                textwrap::termwidth() - 4,
                            ),
                            "  ",
                        )
                        .trim_end(),
                    );
                }

                if !captured_stderr.is_empty() {
                    self.writeln(
                        &mut buffer,
                        &format!("{:—<1$}", "———— Captured stderr: ", textwrap::termwidth()),
                        Color::Red,
                        true,
                    );
                    self.red(
                        &mut buffer,
                        &textwrap::indent(
                            &textwrap::fill(
                                &String::from_utf8_lossy(captured_stderr),
                                textwrap::termwidth() - 4,
                            ),
                            "  ",
                        )
                        .trim_end(),
                    );
                }

                self.writeln(
                    &mut buffer,
                    &format!("{:—<1$}", "", textwrap::termwidth()),
                    Color::Red,
                    true,
                );

                self.fail_count.fetch_add(1, Ordering::SeqCst);
                self.scenarios
                    .write()
                    .unwrap()
                    .insert(scenario.clone(), ScenarioResult::Fail);
            }
            TestResult::Skipped => {
                self.writeln_cmt(
                    &mut buffer,
                    &format!("- {}", msg),
                    cmt,
                    indent,
                    Color::Cyan,
                    false,
                );
                self.print_step_extras(&mut buffer, step);
                self.skipped_count.fetch_add(1, Ordering::SeqCst);
            }
            TestResult::Unimplemented => {
                self.writeln_cmt(
                    &mut buffer,
                    &format!("- {}", msg),
                    cmt,
                    indent,
                    Color::Cyan,
                    false,
                );
                self.print_step_extras(&mut buffer, step);
                self.write(
                    &mut buffer,
                    &format!("{}  ⚡ ", indent),
                    Color::Yellow,
                    false,
                );
                self.println(&mut buffer, "Not yet implemented (skipped)");

                self.skipped_count.fetch_add(1, Ordering::SeqCst);
            }
        };
    }

    fn visit_finish(&self) {
        self.print_finish().unwrap();
    }
}
