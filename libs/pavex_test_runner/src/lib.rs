use std::fmt::Write;
use std::path::{Path, PathBuf};
use std::process::Output;

use ahash::HashMap;
use anyhow::Context;
use console::style;
use libtest_mimic::{Conclusion, Failed};
use toml::toml;
use walkdir::WalkDir;

use itertools::Itertools;
pub use snapshot::print_changeset;

use crate::snapshot::SnapshotTest;

mod snapshot;

/// Return an iterator over the directories containing a UI test.
pub fn get_ui_test_directories(test_folder: &Path) -> impl Iterator<Item = PathBuf> {
    WalkDir::new(test_folder)
        .into_iter()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_name() == "test_config.toml")
        .map(|entry| entry.path().parent().unwrap().to_path_buf())
}

/// Return the name of a UI test given its folder path and the path of the overall UI test folder.
pub fn get_test_name(ui_tests_folder: &Path, ui_test_folder: &Path) -> String {
    ui_test_folder
        .strip_prefix(ui_tests_folder)
        .unwrap()
        .components()
        .filter_map(|c| {
            if let std::path::Component::Normal(s) = c {
                Some(s.to_string_lossy().to_string())
            } else {
                None
            }
        })
        .join("::")
}

/// Create a test case for each folder in `definition_directory`.
///
/// Each test will get a separate runtime environment—a sub-folder of `runtime_directory`. The
/// same sub-folder is reused across multiple test runs to benefit from cargo's incremental compilation.
///
/// Custom configuration can be specified on a per-test basis by including a `test_config.toml` file
/// in the test folder. The available test options are detailed in [`TestConfig`].
///
/// # cargo-nextest
///
/// Our custom test runner is built on top of `libtest_mimic`, which gives us
/// [compatibility out-of-the-box](https://nexte.st/book/custom-test-harnesses.html) with `cargo-nextest`.
pub fn run_tests(
    definition_directory: PathBuf,
    runtime_directory: PathBuf,
) -> Result<Conclusion, anyhow::Error> {
    let arguments = libtest_mimic::Arguments::from_args();

    let cli_profile = std::env::var("PAVEX_TEST_CLI_PROFILE").unwrap_or("debug".to_string());

    let mut tests = Vec::new();
    for entry in get_ui_test_directories(&definition_directory) {
        let name = get_test_name(&definition_directory, &entry);
        let filename = entry.file_name().unwrap();
        let test_data = TestData {
            definition_directory: entry.clone(),
            runtime_directory: runtime_directory.join("tests").join(filename),
        };
        let test_configuration = test_data
            .load_configuration()
            .expect("Failed to load test configuration");
        let is_ignored = test_configuration.ignore;
        let profile = cli_profile.clone();
        let test = libtest_mimic::Trial::test(name.clone(), move || {
            run_test(test_data, test_configuration, profile)
        })
        .with_ignored_flag(is_ignored);
        tests.push(test);
    }
    Ok(libtest_mimic::run(&arguments, tests))
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "snake_case")]
/// Configuration values that can be specified next to the test data to influence how it's going
/// to be executed.
struct TestConfig {
    /// A short description explaining what the test is about, primarily for documentation purposes.
    /// It will be shown in the terminal if the test fails.
    description: String,
    /// Define what we expect to see when running the tests (e.g. should code generation succeed or fail?).
    #[serde(default)]
    expectations: TestExpectations,
    /// Ephemeral crates that should be generated as part of the test setup in order to be
    /// used as dependencies of the main crate under test.
    #[serde(default)]
    ephemeral_dependencies: HashMap<String, EphemeralDependency>,
    /// Crates that should be listed as dependencies of the package under the test, in addition to
    /// `pavex` itself.
    #[serde(default)]
    dependencies: toml::value::Table,
    /// Crates that should be listed as dev dependencies of the test package.
    #[serde(default, rename = "dev-dependencies")]
    dev_dependencies: toml::value::Table,
    /// Ignore the test if set to `true`.
    #[serde(default)]
    ignore: bool,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "snake_case")]
struct EphemeralDependency {
    #[serde(default)]
    /// The name of the package in the generated `Cargo.toml`.
    /// If not specified, the corresponding key in [`TestConfig::ephemeral_dependencies`] will be used.
    package: Option<String>,
    /// The path to the file that should be used as `lib.rs` in the generated library crate.
    path: PathBuf,
    /// Crates that should be listed as dependencies of generated library crate.
    #[serde(default)]
    dependencies: toml::value::Table,
    #[serde(default = "default_ephemeral_version")]
    /// The version of the package in the generated `Cargo.toml`.
    /// If not specified, it defaults to `0.1.0`.
    version: String,
}

fn default_ephemeral_version() -> String {
    "0.1.0".to_string()
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "snake_case")]
struct TestExpectations {
    /// By default, we expect code generation (i.e. `app.build()`) to succeed.
    /// If set to `fail`, the test runner will look for a snapshot of the expected failure message
    /// returned by `pavex` to the user.
    #[serde(default = "ExpectedOutcome::pass")]
    codegen: ExpectedOutcome,
}

impl Default for TestExpectations {
    fn default() -> Self {
        Self {
            codegen: ExpectedOutcome::Pass,
        }
    }
}

#[derive(serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum ExpectedOutcome {
    Pass,
    Fail,
}

impl ExpectedOutcome {
    fn pass() -> ExpectedOutcome {
        ExpectedOutcome::Pass
    }
}

/// Auxiliary data attached to each test definition for convenient retrieval.
/// It's used in [`run_test`].
struct TestData {
    definition_directory: PathBuf,
    runtime_directory: PathBuf,
}

impl TestData {
    /// The directory containing the source code of the project under test—i.e. the blueprint, the generate app
    /// and any integration test, if defined.
    fn test_runtime_directory(&self) -> PathBuf {
        self.runtime_directory.join("project")
    }

    /// The directory containing the source code of all ephemeral dependencies.
    /// 
    /// We don't want to list ephemeral dependencies as members of the workspace of the project under test
    /// in order to be able to have multiple versions of the same crate as dependencies of the project under test.
    /// That would be forbidden by `cargo` if they were listed as members of the same workspace.
    fn ephemeral_deps_runtime_directory(&self) -> PathBuf {
        self.runtime_directory.join("ephemeral_deps")
    }

    fn load_configuration(&self) -> Result<TestConfig, anyhow::Error> {
        let test_config =
            fs_err::read_to_string(self.definition_directory.join("test_config.toml")).context(
                "All UI tests must have an associated `test_config.toml` file with, \
                    at the very least, a `description` field explaining what the test is trying \
                    to verify.",
            )?;
        toml::from_str(&test_config).context(
            "Failed to deserialize `test_config.toml`. Check the file against the expected schema!",
        )
    }

    /// Populate the runtime test folder using the directives and the files in the test
    /// definition folder.
    fn seed_test_filesystem(
        &self,
        test_config: &TestConfig,
        cli_profile: &str,
    ) -> Result<ShouldRunTests, anyhow::Error> {
        let source_directory = self.test_runtime_directory().join("src");
        fs_err::create_dir_all(&source_directory).context(
            "Failed to create the runtime directory for the project under test when setting up the test runtime environment",
        )?;
        fs_err::copy(
            self.definition_directory.join("lib.rs"),
            source_directory.join("lib.rs"),
        )?;

        let deps_subdir = self.ephemeral_deps_runtime_directory();
        fs_err::create_dir_all(&source_directory).context(
            "Failed to create the runtime directory for ephemeral dependencies when setting up the test runtime environment",
        )?;

        for (dependency_name, dependency_config) in &test_config.ephemeral_dependencies {
            let dep_runtime_directory = deps_subdir.join(dependency_name);
            let package_name = dependency_config
                .package
                .clone()
                .unwrap_or(dependency_name.to_owned());
            let dep_source_directory = dep_runtime_directory.join("src");
            fs_err::create_dir_all(&dep_source_directory).context(
                "Failed to create the source directory for an ephemeral dependency when setting up the test runtime environment",
            )?;
            fs_err::copy(
                self.definition_directory.join(&dependency_config.path),
                dep_source_directory.join("lib.rs"),
            )?;
            let mut cargo_toml = toml! {
                [package]
                name = "dummy"
                version = "0.1.0"
                edition = "2021"

                [dependencies]
                pavex_builder = { path = "../../../../../../libs/pavex_builder" }
                pavex_runtime = { path = "../../../../../../libs/pavex_runtime" }
            };
            cargo_toml["package"]["name"] = package_name.into();
            cargo_toml["package"]["version"] = dependency_config.version.clone().into();
            let deps = cargo_toml
                .get_mut("dependencies")
                .unwrap()
                .as_table_mut()
                .unwrap();
            deps.extend(dependency_config.dependencies.clone());

            fs_err::write(
                dep_runtime_directory.join("Cargo.toml"),
                toml::to_string(&cargo_toml)?,
            )?;
        }

        let integration_test_file = self.definition_directory.join("test.rs");
        let has_tests = integration_test_file.exists();
        if has_tests {
            let integration_test_directory = self.test_runtime_directory().join("integration");
            let integration_test_src_directory = integration_test_directory.join("src");
            let integration_test_test_directory = integration_test_directory.join("tests");
            fs_err::create_dir_all(&integration_test_src_directory).context(
                "Failed to create the runtime directory for integration tests when setting up the test runtime environment",
            )?;
            fs_err::create_dir_all(&integration_test_test_directory).context(
                "Failed to create the runtime directory for integration tests when setting up the test runtime environment",
            )?;
            fs_err::copy(
                integration_test_file,
                integration_test_test_directory.join("run.rs"),
            )?;
            fs_err::write(integration_test_src_directory.join("lib.rs"), "")?;

            let mut cargo_toml = toml! {
                [package]
                name = "integration"
                version = "0.1.0"
                edition = "2021"

                [dependencies]
                application = { path = "../generated_app" }

                [dev-dependencies]
                tokio = { version = "1", features = ["full"] }
                reqwest = "0.11"
                pavex_runtime = { path = "../../../../../../libs/pavex_runtime" }
            };

            let dev_deps = cargo_toml
                .get_mut("dev-dependencies")
                .unwrap()
                .as_table_mut()
                .unwrap();
            dev_deps.extend(test_config.dev_dependencies.clone());

            fs_err::write(
                integration_test_directory.join("Cargo.toml"),
                toml::to_string(&cargo_toml)?,
            )?;
        }

        // Dummy application crate, ahead of code generation.
        {
            let application_dir = self.test_runtime_directory().join("generated_app");
            let application_src_dir = application_dir.join("src");
            fs_err::create_dir_all(&application_src_dir).context(
                "Failed to create the runtime directory for the generated application when setting up the test runtime environment",
            )?;
            fs_err::write(application_src_dir.join("lib.rs"), "")?;

            let cargo_toml = toml! {
                [package]
                name = "application"
                version = "0.1.0"
                edition = "2021"
            };
            fs_err::write(
                application_dir.join("Cargo.toml"),
                toml::to_string(&cargo_toml)?,
            )?;
        }

        let mut cargo_toml = toml! {
            [workspace]
            members = [".", "generated_app"]

            [package]
            name = "app"
            version = "0.1.0"
            edition = "2021"

            [dependencies]
            pavex_builder = { path = "../../../../../libs/pavex_builder" }
            pavex_runtime = { path = "../../../../../libs/pavex_runtime" }
        };
        if has_tests {
            cargo_toml["workspace"]["members"]
                .as_array_mut()
                .unwrap()
                .push("integration".into());
        }
        let deps = cargo_toml
            .get_mut("dependencies")
            .unwrap()
            .as_table_mut()
            .unwrap();
        deps.extend(test_config.dependencies.clone());
        let ephemeral_dependencies =
            test_config
                .ephemeral_dependencies
                .iter()
                .map(|(key, config)| {
                    let mut value = toml::value::Table::new();
                    value.insert("path".into(), format!("../ephemeral_deps/{key}").into());
                    if let Some(package_name) = config.package.as_ref() {
                        value.insert("package".into(), package_name.clone().into());
                    }
                    (key.to_owned(), toml::Value::Table(value))
                });
        deps.extend(ephemeral_dependencies);

        fs_err::write(
            self.test_runtime_directory().join("Cargo.toml"),
            toml::to_string(&cargo_toml)?,
        )?;

        // Use sccache to avoid rebuilding the same dependencies
        // over and over again.
        let cargo_config = toml! {
            [build]
            rustc-wrapper = "sccache"
        };
        let dot_cargo_folder = self.runtime_directory.join(".cargo");
        fs_err::create_dir_all(&dot_cargo_folder)?;
        fs_err::write(
            dot_cargo_folder.join("config.toml"),
            toml::to_string(&cargo_config)?,
        )?;

        let main_rs = format!(
            r#"use app::blueprint;
use std::str::FromStr;

fn main() -> Result<(), Box<dyn std::error::Error>> {{
    let path = std::path::PathBuf::from_str("blueprint.json")?;
    blueprint().persist(&path)?;

    let status = std::process::Command::new("../../../../../libs/target/{cli_profile}/pavex_cli")
        .arg("--color")
        .arg("always")
        .arg("generate")
        .arg("-b")
        .arg(&path)
        .arg("--diagnostics")
        .arg("diagnostics.dot")
        .arg("-o")
        .arg("generated_app")
        .status()?;
       
    if !status.success() {{
        std::process::exit(1); 
    }}
     
    Ok(())
}}"#
        );
        fs_err::write(source_directory.join("main.rs"), main_rs)?;
        Ok(if has_tests {
            ShouldRunTests::Yes
        } else {
            ShouldRunTests::No
        })
    }
}

enum ShouldRunTests {
    Yes,
    No,
}

fn run_test(test: TestData, config: TestConfig, cli_profile: String) -> Result<(), Failed> {
    match _run_test(&config, &test, &cli_profile) {
        Ok(TestOutcome {
            outcome: Err(mut msg),
            codegen_output,
            compilation_output,
            test_output,
        }) => Err(Failed::from({
            write!(
                &mut msg,
                "\n\nCODEGEN:\n\t--- STDOUT:\n{}\n\t--- STDERR:\n{}",
                codegen_output.stdout, codegen_output.stderr
            )
            .unwrap();
            if let Some(compilation_output) = compilation_output {
                write!(
                    &mut msg,
                    "\n\nCARGO CHECK:\n\t--- STDOUT:\n{}\n\t--- STDERR:\n{}",
                    compilation_output.stdout, compilation_output.stderr
                )
                .unwrap();
            }
            if let Some(test_output) = test_output {
                write!(
                    &mut msg,
                    "\n\nCARGO TEST:\n\t--- STDOUT:\n{}\n\t--- STDERR:\n{}",
                    test_output.stdout, test_output.stderr
                )
                .unwrap();
            }
            enrich_failure_message(&config, msg)
        })),
        Err(e) => Err(Failed::from(enrich_failure_message(
            &config,
            unexpected_failure_message(&e),
        ))),
        Ok(TestOutcome {
            outcome: Ok(()), ..
        }) => Ok(()),
    }
}

fn _run_test(
    test_config: &TestConfig,
    test: &TestData,
    cli_profile: &str,
) -> Result<TestOutcome, anyhow::Error> {
    let should_run_tests = test
        .seed_test_filesystem(test_config, cli_profile)
        .context("Failed to seed the filesystem for the test runtime folder")?;

    // Generate the application code
    let output = std::process::Command::new("cargo")
        .env("RUSTFLAGS", "-Awarnings")
        .arg("run")
        .arg("--quiet")
        .current_dir(&test.test_runtime_directory())
        .output()
        .context("Failed to perform code generation")?;

    let codegen_output: CommandOutput = (&output).try_into()?;

    let expectations_directory = test.definition_directory.join("expectations");

    if !output.status.success() {
        return match test_config.expectations.codegen {
            ExpectedOutcome::Pass => Ok(TestOutcome {
                outcome: Err("We failed to generate the application code.".to_string()),
                codegen_output,
                compilation_output: None,
                test_output: None,
            }),
            ExpectedOutcome::Fail => {

                let stderr_snapshot = SnapshotTest::new(expectations_directory.join("stderr.txt"));
                if stderr_snapshot.verify(&codegen_output.stderr).is_err() {
                    return Ok(TestOutcome {
                        outcome: Err("The failure message returned by code generation doesn't match what we expected".into()),
                        codegen_output,
                        compilation_output: None,
                        test_output: None,
                    });
                }
                Ok(TestOutcome {
                    outcome: Ok(()),
                    codegen_output,
                    compilation_output: None,
                    test_output: None,
                })
            }
        };
    } else if ExpectedOutcome::Fail == test_config.expectations.codegen {
        return Ok(TestOutcome {
            outcome: Err("We expected code generation to fail, but it succeeded!".into()),
            codegen_output,
            compilation_output: None,
            test_output: None,
        });
    };

    let diagnostics_snapshot = SnapshotTest::new(expectations_directory.join("diagnostics.dot"));
    let actual_diagnostics =
        fs_err::read_to_string(test.test_runtime_directory().join("diagnostics.dot"))?;
    // We don't exit early here to get the generated code snapshot as well.
    // This allows to update both code snapshot and diagnostics snapshot in one go via
    // `cargo r --bin snaps` for a failing test instead of having to do them one at a time,
    // with a test run in the middle.
    let diagnostics_outcome = diagnostics_snapshot.verify(&actual_diagnostics);

    let app_code_snapshot = SnapshotTest::new(expectations_directory.join("app.rs"));
    let actual_app_code = fs_err::read_to_string(
        test.test_runtime_directory()
            .join("generated_app")
            .join("src")
            .join("lib.rs"),
    )
    .unwrap();
    let codegen_outcome = app_code_snapshot.verify(&actual_app_code);

    // Check that the generated code compiles
    let output = std::process::Command::new("cargo")
        .env("RUSTFLAGS", "-Awarnings")
        .arg("check")
        .arg("-p")
        .arg("application")
        .arg("--quiet")
        .current_dir(&test.test_runtime_directory())
        .output()
        .unwrap();
    let compilation_output: Result<CommandOutput, _> = (&output).try_into();

    if diagnostics_outcome.is_err() {
        return Ok(TestOutcome {
            outcome: Err(
                "Diagnostics for the generated application don't match what we expected".into(),
            ),
            codegen_output,
            compilation_output: None,
            test_output: None,
        });
    }

    if codegen_outcome.is_err() {
        return Ok(TestOutcome {
            outcome: Err("The generated application code doesn't match what we expected".into()),
            codegen_output,
            compilation_output: None,
            test_output: None,
        });
    }

    let compilation_output = compilation_output?;
    if !output.status.success() {
        return Ok(TestOutcome {
            outcome: Err("The generated application code doesn't compile.".into()),
            codegen_output,
            compilation_output: Some(compilation_output),
            test_output: None,
        });
    }

    // Run integration tests, if we have any,
    if let ShouldRunTests::Yes = should_run_tests {
        let output = std::process::Command::new("cargo")
            .env("RUSTFLAGS", "-Awarnings")
            .arg("t")
            .arg("-p")
            .arg("integration")
            .current_dir(&test.test_runtime_directory())
            .output()
            .unwrap();
        let test_output: CommandOutput = (&output).try_into()?;
        if !output.status.success() {
            return Ok(TestOutcome {
                outcome: Err("Integration tests failed.".into()),
                codegen_output,
                test_output: Some(test_output),
                compilation_output: Some(compilation_output),
            });
        }
    }

    Ok(TestOutcome {
        outcome: Ok(()),
        codegen_output,
        compilation_output: Some(compilation_output),
        test_output: None,
    })
}

struct TestOutcome {
    outcome: Result<(), String>,
    codegen_output: CommandOutput,
    compilation_output: Option<CommandOutput>,
    test_output: Option<CommandOutput>,
}

/// A refined `std::process::Output` that assumes that both stderr and stdout are valid UTF8.
struct CommandOutput {
    stdout: String,
    stderr: String,
}

impl TryFrom<&Output> for CommandOutput {
    type Error = anyhow::Error;

    fn try_from(o: &Output) -> Result<Self, Self::Error> {
        let stdout = std::str::from_utf8(&o.stdout)
            .context("The application printed invalid UTF8 data to stdout")?;
        let stderr = std::str::from_utf8(&o.stderr)
            .context("The application printed invalid UTF8 data to stderr")?;
        Ok(Self {
            stdout: stdout.to_string(),
            stderr: stderr.to_string(),
        })
    }
}

fn unexpected_failure_message(e: &anyhow::Error) -> String {
    format!(
        "An unexpected error was encountered when running a test.\n\n{}\n---\n{:?}",
        &e, &e
    )
}

fn enrich_failure_message(config: &TestConfig, error: impl AsRef<str>) -> String {
    let description = style(textwrap::indent(&config.description, "    ")).cyan();
    let error = style(textwrap::indent(error.as_ref(), "    ")).red();
    format!(
        "{}\n{description}.\n{}\n{error}",
        style("What is the test about:").cyan().dim().bold(),
        style("What went wrong:").red().bold(),
    )
}
