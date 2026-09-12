//! HCLI's direct API-key setup, before the general provider onboarding flow.

use anyhow::{Context, Result};
use std::io::{self, IsTerminal, Write};

use super::args::Args;
use super::commands::{ProviderAddOptions, configure_provider_profile};
use super::provider_init::ProviderChoice;
use crate::config::Config;
use crate::provider_catalog::{load_api_key_from_env_or_config, save_env_value_to_env_file};

const PROFILE: &str = "hcap";
const API_BASE: &str = "https://hcap.ai/v1";
const DEFAULT_MODEL: &str = "gpt-6-astra";
const KEY_ENV: &str = "HCAP_API_KEY";
const KEY_FILE: &str = "hcap.env";

/// Keep the fork's credentials and daemon separate from an installed Jcode.
/// Explicit environment/CLI overrides still support sandboxes and remote sockets.
pub(crate) fn initialize_paths() -> Result<()> {
    crate::env::set_var("HCLI_UI", "1");
    if std::env::var_os("JCODE_HOME").is_none() {
        let home = dirs::home_dir().context("Cannot locate the home directory for HCLI")?;
        crate::env::set_var("JCODE_HOME", home.join(".hcli"));
    }
    if std::env::var_os("JCODE_RUNTIME_DIR").is_none() {
        // The daemon lock and other runtime files are directory-scoped, not
        // socket-scoped. A different socket alone still collides with Jcode.
        crate::env::set_var(
            "JCODE_RUNTIME_DIR",
            crate::storage::runtime_dir().join("hcli"),
        );
    }
    if std::env::var_os("JCODE_SOCKET").is_none() {
        crate::env::set_var(
            "JCODE_SOCKET",
            crate::storage::runtime_dir().join("hcli.sock"),
        );
    }
    Ok(())
}

fn needs_hcap_startup(args: &Args) -> bool {
    args.command.is_none()
        && args.ssh.is_none()
        && !args.onboarding_sim
        && !args.update_sim
        && args.provider == ProviderChoice::Auto
        && args.provider_profile.is_none()
}

pub(crate) fn prepare_startup(args: &mut Args) -> Result<()> {
    if !needs_hcap_startup(args) {
        return Ok(());
    }

    let key = match load_api_key_from_env_or_config(KEY_ENV, KEY_FILE) {
        Some(key) => {
            if io::stderr().is_terminal() {
                print_welcome(false);
            }
            key
        }
        None => {
            if !io::stdin().is_terminal() || !io::stderr().is_terminal() {
                anyhow::bail!(
                    "Set HCAP_API_KEY or launch HCLI in a terminal to enter your hcap.ai API key."
                );
            }
            print_welcome(true);
            loop {
                eprint!("Enter your hcap.ai API key: ");
                io::stderr().flush()?;
                let key = super::login::read_secret_line()?;
                if !key.is_empty() {
                    // Save before continuing; Ctrl+C and empty input never create credentials.
                    save_env_value_to_env_file(KEY_ENV, KEY_FILE, Some(&key))?;
                    eprintln!("\n  API key saved. You're set for next time.");
                    eprintln!("  Opening your workspace...\n");
                    break key;
                }
                eprintln!("Please enter an API key, or press Ctrl+C to cancel.");
            }
        }
    };

    ensure_profile()?;
    crate::env::set_var(KEY_ENV, key);
    crate::config::invalidate_config_cache();
    // Explicit selection carries the profile into daemon startup.
    args.provider_profile = Some(PROFILE.to_string());
    Ok(())
}

/// Refresh rich catalog details in this client as well as the daemon. A daemon
/// from an older build may have cached only names/prices, dropping `extra`.
pub(crate) fn refresh_catalog_in_background() {
    let config = crate::config::config();
    let Some(profile) = config
        .providers
        .get(PROFILE)
        .filter(|profile| profile.model_catalog)
    else {
        return;
    };
    let Ok(provider) =
        jcode_provider_openrouter_runtime::OpenRouterProvider::new_named_openai_compatible(
            PROFILE, profile,
        )
    else {
        return;
    };
    tokio::spawn(async move {
        if matches!(
            tokio::time::timeout(
                std::time::Duration::from_secs(15),
                provider.refresh_models()
            )
            .await,
            Ok(Ok(_))
        ) {
            crate::bus::Bus::global().publish_models_updated();
        }
    });
}

fn print_welcome(needs_key: bool) {
    let color = std::env::var_os("NO_COLOR").is_none()
        && std::env::var("TERM").is_ok_and(|term| term != "dumb");
    let (accent, bold, dim, reset) = if color {
        ("\x1b[36m", "\x1b[1m", "\x1b[2m", "\x1b[0m")
    } else {
        ("", "", "", "")
    };
    eprintln!("\n  {accent}{bold}HCLI{reset}  {dim}/  hcap.ai{reset}");
    eprintln!("  {accent}──────────────────────────────{reset}");
    if !needs_key {
        eprintln!("  {dim}Opening your workspace...{reset}\n");
        return;
    }
    eprintln!("  Your workspace, ready to build.\n");
    eprintln!("  Get an API key at https://hcap.ai");
    eprintln!("  {dim}Input is hidden · Ctrl+C to cancel{reset}\n");
}

fn ensure_profile() -> Result<()> {
    let path = Config::path().context("No config path for HCLI setup")?;
    let existing = match std::fs::read_to_string(&path) {
        Ok(content) => toml::from_str::<Config>(&content).context("Cannot read HCLI config")?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => Config::default(),
        Err(error) => return Err(error).context("Cannot read HCLI config"),
    };
    if existing.providers.contains_key(PROFILE) {
        return Ok(());
    }

    configure_provider_profile(ProviderAddOptions {
        name: PROFILE.to_string(),
        base_url: API_BASE.to_string(),
        model: DEFAULT_MODEL.to_string(),
        context_window: None,
        api_key_env: Some(KEY_ENV.to_string()),
        api_key: None,
        api_key_stdin: false,
        no_api_key: false,
        auth: None,
        auth_header: None,
        env_file: Some(KEY_FILE.to_string()),
        set_default: true,
        overwrite: false,
        provider_routing: false,
        model_catalog: true,
        json: false,
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    struct EnvGuard(&'static str, Option<std::ffi::OsString>);
    impl EnvGuard {
        fn set(key: &'static str, value: impl AsRef<std::ffi::OsStr>) -> Self {
            let previous = std::env::var_os(key);
            crate::env::set_var(key, value);
            Self(key, previous)
        }
    }
    impl Drop for EnvGuard {
        fn drop(&mut self) {
            if let Some(value) = &self.1 {
                crate::env::set_var(self.0, value);
            } else {
                crate::env::remove_var(self.0);
            }
        }
    }

    #[test]
    fn startup_only_intercepts_default_local_launches() {
        for argv in [vec!["jcode"], vec!["jcode", "--model", "custom-model"]] {
            assert!(needs_hcap_startup(&Args::try_parse_from(argv).unwrap()));
        }
        for argv in [
            vec!["jcode", "--provider", "openai"],
            vec!["jcode", "--provider-profile", "my-api"],
            vec!["jcode", "--ssh", "example.com"],
            vec!["jcode", "--onboarding-sim"],
            vec!["jcode", "--update-sim"],
            vec!["jcode", "run", "hello"],
            vec!["jcode", "serve"],
        ] {
            assert!(!needs_hcap_startup(&Args::try_parse_from(argv).unwrap()));
        }
    }

    #[test]
    fn fork_paths_are_separate_and_respect_overrides() {
        let _lock = crate::storage::lock_test_env();
        let temp = tempfile::tempdir().unwrap();
        let _ui = EnvGuard::set("HCLI_UI", "");
        let _home = EnvGuard::set("JCODE_HOME", "");
        let _socket = EnvGuard::set("JCODE_SOCKET", "");
        let _runtime = EnvGuard::set("JCODE_RUNTIME_DIR", temp.path());
        crate::env::remove_var("JCODE_HOME");
        crate::env::remove_var("JCODE_SOCKET");
        crate::env::remove_var("JCODE_RUNTIME_DIR");
        let expected_runtime = crate::storage::runtime_dir().join("hcli");
        initialize_paths().unwrap();
        assert_eq!(crate::storage::runtime_dir(), expected_runtime);
        assert_eq!(
            crate::storage::jcode_dir().unwrap(),
            dirs::home_dir().unwrap().join(".hcli")
        );
        assert_eq!(
            std::env::var_os("JCODE_SOCKET").unwrap(),
            expected_runtime.join("hcli.sock")
        );
        crate::env::set_var("JCODE_HOME", temp.path());
        crate::env::set_var("JCODE_RUNTIME_DIR", temp.path());
        crate::env::set_var("JCODE_SOCKET", temp.path().join("custom.sock"));
        initialize_paths().unwrap();
        assert_eq!(crate::storage::jcode_dir().unwrap(), temp.path());
        assert_eq!(crate::storage::runtime_dir(), temp.path());
        assert_eq!(
            std::env::var_os("JCODE_SOCKET").unwrap(),
            temp.path().join("custom.sock")
        );
    }

    #[test]
    fn environment_key_skips_prompt_without_persisting_secret() {
        let _lock = crate::storage::lock_test_env();
        let temp = tempfile::tempdir().unwrap();
        let _home = EnvGuard::set("JCODE_HOME", temp.path());
        let _key = EnvGuard::set(KEY_ENV, "test-env-key");
        let mut args = Args::try_parse_from(["jcode"]).unwrap();
        prepare_startup(&mut args).unwrap();
        assert_eq!(args.provider_profile.as_deref(), Some(PROFILE));
        assert!(
            !crate::storage::app_config_dir()
                .unwrap()
                .join(KEY_FILE)
                .exists()
        );
        assert!(
            !std::fs::read_to_string(Config::path().unwrap())
                .unwrap()
                .contains("test-env-key")
        );
    }

    #[test]
    fn profile_setup_preserves_existing_settings_and_uses_separate_key_storage() {
        let _lock = crate::storage::lock_test_env();
        let temp = tempfile::tempdir().unwrap();
        let _home = EnvGuard::set("JCODE_HOME", temp.path());
        let path = Config::path().unwrap();
        std::fs::write(&path, "# keep my settings\n[display]\nemoji = false\n").unwrap();
        ensure_profile().unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        let config: Config = toml::from_str(&content).unwrap();
        assert!(content.contains("# keep my settings"));
        assert!(!config.display.emoji);
        assert_eq!(config.provider.default_provider.as_deref(), Some(PROFILE));
        let profile = &config.providers[PROFILE];
        assert_eq!(profile.base_url, API_BASE);
        assert_eq!(profile.api_key_env.as_deref(), Some(KEY_ENV));
        assert_eq!(profile.env_file.as_deref(), Some(KEY_FILE));
        assert!(profile.api_key.is_none());
        assert!(profile.model_catalog);
        assert_eq!(profile.default_model.as_deref(), Some(DEFAULT_MODEL));
        ensure_profile().unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), content);
    }

    #[test]
    fn returning_launch_reuses_saved_key_without_prompting() {
        let _lock = crate::storage::lock_test_env();
        let temp = tempfile::tempdir().unwrap();
        let _home = EnvGuard::set("JCODE_HOME", temp.path());
        let _key = EnvGuard::set(KEY_ENV, "");
        save_env_value_to_env_file(KEY_ENV, KEY_FILE, Some("test-hcap-key")).unwrap();
        crate::env::remove_var(KEY_ENV);
        let mut args = Args::try_parse_from(["jcode"]).unwrap();
        prepare_startup(&mut args).unwrap();
        assert_eq!(args.provider_profile.as_deref(), Some(PROFILE));
        assert_eq!(std::env::var(KEY_ENV).unwrap(), "test-hcap-key");
        let content = std::fs::read_to_string(Config::path().unwrap()).unwrap();
        assert!(!content.contains("test-hcap-key"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let path = crate::storage::app_config_dir().unwrap().join(KEY_FILE);
            assert_eq!(
                std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }
}
