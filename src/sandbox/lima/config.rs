//! Lima configuration YAML generation.

use anyhow::Result;
use serde_yaml::Value;

use super::mounts::Mount;
use crate::config::SandboxConfig;

/// Generate Lima configuration YAML.
pub fn generate_lima_config(
    _instance_name: &str,
    mounts: &[Mount],
    sandbox_config: &SandboxConfig,
) -> Result<String> {
    let mut config = serde_yaml::Mapping::new();

    // Use custom image if configured, otherwise default to minimal Debian 12
    // Debian genericcloud images are ~330MB vs Ubuntu's ~600MB
    let arch = std::env::consts::ARCH;
    let image_arch = if arch == "aarch64" || arch == "arm64" {
        "aarch64"
    } else {
        "x86_64"
    };

    let mut image_config = serde_yaml::Mapping::new();
    if let Some(custom_image) = &sandbox_config.image {
        image_config.insert("location".into(), custom_image.as_str().into());
    } else {
        let default_url = if image_arch == "aarch64" {
            "https://cloud.debian.org/images/cloud/bookworm/latest/debian-12-genericcloud-arm64.qcow2"
        } else {
            "https://cloud.debian.org/images/cloud/bookworm/latest/debian-12-genericcloud-amd64.qcow2"
        };
        image_config.insert("location".into(), default_url.into());
        image_config.insert("arch".into(), image_arch.into());
    }

    config.insert("images".into(), vec![Value::Mapping(image_config)].into());

    // Use VZ backend on macOS (fastest), QEMU on Linux
    #[cfg(target_os = "macos")]
    {
        config.insert("vmType".into(), "vz".into());

        // Enable Rosetta for x86 binaries on ARM (use new nested format)
        if arch == "aarch64" || arch == "arm64" {
            let mut rosetta = serde_yaml::Mapping::new();
            rosetta.insert("enabled".into(), true.into());
            rosetta.insert("binfmt".into(), true.into());

            let mut vz = serde_yaml::Mapping::new();
            vz.insert("rosetta".into(), rosetta.into());

            let mut vm_opts = serde_yaml::Mapping::new();
            vm_opts.insert("vz".into(), vz.into());

            config.insert("vmOpts".into(), vm_opts.into());
        }
    }

    #[cfg(not(target_os = "macos"))]
    {
        config.insert("vmType".into(), "qemu".into());
    }

    // Resource allocation
    config.insert("cpus".into(), Value::Number(sandbox_config.cpus().into()));
    config.insert("memory".into(), sandbox_config.memory().into());
    config.insert("disk".into(), sandbox_config.disk().into());

    // CRITICAL: Disable containerd (saves 30-40 seconds boot time)
    let mut containerd = serde_yaml::Mapping::new();
    containerd.insert("system".into(), false.into());
    containerd.insert("user".into(), false.into());
    config.insert("containerd".into(), containerd.into());

    // Generate mounts
    let mount_list: Vec<Value> = mounts
        .iter()
        .map(|m| {
            let mut mount_config = serde_yaml::Mapping::new();
            mount_config.insert(
                "location".into(),
                m.host_path.to_string_lossy().to_string().into(),
            );
            mount_config.insert("writable".into(), (!m.read_only).into());

            if m.host_path != m.guest_path {
                mount_config.insert(
                    "mountPoint".into(),
                    m.guest_path.to_string_lossy().to_string().into(),
                );
            }

            Value::Mapping(mount_config)
        })
        .collect();
    config.insert("mounts".into(), mount_list.into());

    // Provision scripts (run on first VM creation only)
    let mut provisions = Vec::new();

    if !sandbox_config.skip_default_provision() {
        let system_script = r#"#!/bin/bash
set -eux
apt-get update
apt-get install -y --no-install-recommends curl ca-certificates git xz-utils
"#;

        let user_script = r#"#!/bin/bash
set -eux
curl -fsSL https://claude.ai/install.sh | bash
curl -fsSL https://raw.githubusercontent.com/raine/workmux/main/scripts/install.sh | bash
# Ensure ~/.local/bin is on PATH for non-interactive shells
echo 'export PATH="$HOME/.local/bin:$PATH"' >> ~/.profile

# Install Nix via Determinate Systems installer (needs root for /nix)
if ! command -v nix >/dev/null 2>&1; then
    curl --proto '=https' --tlsv1.2 -sSf -L https://install.determinate.systems/nix | \
      sudo sh -s -- install linux --init none --no-confirm
    # Single-user VM: make nix store writable by the user so nix/devbox
    # can install packages without root
    sudo chown -R "$(id -u):$(id -g)" /nix
fi

# Source nix profile for this script and future login shells
if [ -f /nix/var/nix/profiles/default/etc/profile.d/nix-daemon.sh ]; then
    . /nix/var/nix/profiles/default/etc/profile.d/nix-daemon.sh
fi
if ! grep -q 'nix-daemon.sh' ~/.profile 2>/dev/null; then
    echo '. /nix/var/nix/profiles/default/etc/profile.d/nix-daemon.sh' >> ~/.profile
fi

# Install Devbox (needs root for /usr/local/bin)
if ! command -v devbox >/dev/null 2>&1; then
    curl -fsSL https://get.jetify.com/devbox | sudo bash -s -- -f
    # Launcher script has execute-only perms; bash needs read permission
    sudo chmod +r /usr/local/bin/devbox
    # Trigger download of the real binary during provisioning
    devbox version
fi

# Symlink Claude config from mounted state directory (seeded from host)
# This preserves onboarding state, tips history, etc. across VM recreations
ln -sfn "$HOME/.workmux-state/.claude.json" "$HOME/.claude.json"

# Install afplay shim that routes through workmux RPC to host
cat > ~/.local/bin/afplay << 'SHIM'
#!/bin/sh
# Shim that forwards afplay calls to the host via workmux RPC
exec workmux notify sound "$@"
SHIM
chmod +x ~/.local/bin/afplay
"#;

        let mut system_provision = serde_yaml::Mapping::new();
        system_provision.insert("mode".into(), "system".into());
        system_provision.insert("script".into(), system_script.into());

        let mut user_provision = serde_yaml::Mapping::new();
        user_provision.insert("mode".into(), "user".into());
        user_provision.insert("script".into(), user_script.into());

        provisions.push(Value::Mapping(system_provision));
        provisions.push(Value::Mapping(user_provision));
    }

    if let Some(script) = sandbox_config.provision_script() {
        let mut custom_provision = serde_yaml::Mapping::new();
        custom_provision.insert("mode".into(), "user".into());
        custom_provision.insert("script".into(), script.into());
        provisions.push(Value::Mapping(custom_provision));
    }

    config.insert("provision".into(), provisions.into());

    Ok(serde_yaml::to_string(&config)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_generate_lima_config() {
        let mounts = vec![
            Mount::rw(PathBuf::from("/Users/test/code")),
            Mount {
                host_path: PathBuf::from("/Users/test/.claude"),
                guest_path: PathBuf::from("/root/.claude"),
                read_only: false,
            },
        ];

        let sandbox_config = SandboxConfig::default();
        let yaml = generate_lima_config("test-vm", &mounts, &sandbox_config).unwrap();

        // Basic sanity checks
        assert!(yaml.contains("images:"));
        assert!(yaml.contains("mounts:"));
        assert!(yaml.contains("/Users/test/code"));
        assert!(yaml.contains("containerd:"));
        assert!(yaml.contains("provision:"));
        assert!(yaml.contains("cpus: 4"));
        assert!(yaml.contains("memory: 4GiB"));
        assert!(yaml.contains("disk: 100GiB"));
    }

    #[test]
    fn test_generate_lima_config_provision_scripts() {
        let mounts = vec![Mount::rw(PathBuf::from("/tmp/test"))];
        let sandbox_config = SandboxConfig::default();
        let yaml = generate_lima_config("test-vm", &mounts, &sandbox_config).unwrap();

        // System provision installs dependencies
        assert!(yaml.contains("mode: system"));
        assert!(yaml.contains("apt-get install"));
        assert!(yaml.contains("curl"));
        assert!(yaml.contains("git"));
        assert!(yaml.contains("xz-utils"));

        // User provision installs Claude Code and workmux
        assert!(yaml.contains("mode: user"));
        assert!(yaml.contains("claude.ai/install.sh"));
        assert!(yaml.contains("workmux/main/scripts/install.sh"));

        // User provision installs Nix and Devbox
        assert!(yaml.contains("install.determinate.systems/nix"));
        assert!(yaml.contains("get.jetify.com/devbox"));

        // User provision symlinks Claude config from state directory
        assert!(
            yaml.contains(r#"ln -sfn "$HOME/.workmux-state/.claude.json" "$HOME/.claude.json""#)
        );
    }

    #[test]
    fn test_generate_lima_config_default_provision_count() {
        let mounts = vec![Mount::rw(PathBuf::from("/tmp/test"))];
        let sandbox_config = SandboxConfig::default();
        let yaml = generate_lima_config("test-vm", &mounts, &sandbox_config).unwrap();

        let parsed: serde_yaml::Value = serde_yaml::from_str(&yaml).unwrap();
        let provisions = parsed["provision"].as_sequence().unwrap();
        assert_eq!(provisions.len(), 2, "default should have 2 provision steps");
    }

    #[test]
    fn test_generate_lima_config_custom_provision() {
        let mounts = vec![Mount::rw(PathBuf::from("/tmp/test"))];
        let sandbox_config = SandboxConfig {
            provision: Some("sudo apt-get install -y ripgrep\necho done".to_string()),
            ..Default::default()
        };
        let yaml = generate_lima_config("test-vm", &mounts, &sandbox_config).unwrap();

        let parsed: serde_yaml::Value = serde_yaml::from_str(&yaml).unwrap();
        let provisions = parsed["provision"].as_sequence().unwrap();
        assert_eq!(
            provisions.len(),
            3,
            "should have 3 provision steps with custom script"
        );

        let custom = &provisions[2];
        assert_eq!(custom["mode"].as_str().unwrap(), "user");
        let script = custom["script"].as_str().unwrap();
        assert!(script.contains("sudo apt-get install -y ripgrep"));
        assert!(script.contains("echo done"));
    }

    #[test]
    fn test_generate_lima_config_custom_image() {
        let mounts = vec![Mount::rw(PathBuf::from("/tmp/test"))];
        let sandbox_config = SandboxConfig {
            image: Some("file:///Users/me/.lima/images/workmux-golden.qcow2".to_string()),
            ..Default::default()
        };
        let yaml = generate_lima_config("test-vm", &mounts, &sandbox_config).unwrap();

        let parsed: serde_yaml::Value = serde_yaml::from_str(&yaml).unwrap();
        let images = parsed["images"].as_sequence().unwrap();
        let image = &images[0];
        assert_eq!(
            image["location"].as_str().unwrap(),
            "file:///Users/me/.lima/images/workmux-golden.qcow2"
        );
        // Custom images should not have arch set (user provides arch-appropriate image)
        assert!(image["arch"].is_null());
    }

    #[test]
    fn test_generate_lima_config_default_image() {
        let mounts = vec![Mount::rw(PathBuf::from("/tmp/test"))];
        let sandbox_config = SandboxConfig::default();
        let yaml = generate_lima_config("test-vm", &mounts, &sandbox_config).unwrap();

        let parsed: serde_yaml::Value = serde_yaml::from_str(&yaml).unwrap();
        let images = parsed["images"].as_sequence().unwrap();
        let image = &images[0];
        let location = image["location"].as_str().unwrap();
        assert!(location.contains("debian-12-genericcloud"));
        assert!(image["arch"].as_str().is_some());
    }

    #[test]
    fn test_generate_lima_config_skip_default_provision() {
        let mounts = vec![Mount::rw(PathBuf::from("/tmp/test"))];
        let sandbox_config = SandboxConfig {
            skip_default_provision: Some(true),
            ..Default::default()
        };
        let yaml = generate_lima_config("test-vm", &mounts, &sandbox_config).unwrap();

        let parsed: serde_yaml::Value = serde_yaml::from_str(&yaml).unwrap();
        let provisions = parsed["provision"].as_sequence().unwrap();
        assert_eq!(
            provisions.len(),
            0,
            "should have no provision steps when skipping defaults"
        );
        assert!(!yaml.contains("apt-get"));
        assert!(!yaml.contains("claude.ai/install.sh"));
    }

    #[test]
    fn test_generate_lima_config_skip_default_provision_with_custom() {
        let mounts = vec![Mount::rw(PathBuf::from("/tmp/test"))];
        let sandbox_config = SandboxConfig {
            skip_default_provision: Some(true),
            provision: Some("echo custom setup".to_string()),
            ..Default::default()
        };
        let yaml = generate_lima_config("test-vm", &mounts, &sandbox_config).unwrap();

        let parsed: serde_yaml::Value = serde_yaml::from_str(&yaml).unwrap();
        let provisions = parsed["provision"].as_sequence().unwrap();
        assert_eq!(
            provisions.len(),
            1,
            "should have only custom provision step"
        );

        let custom = &provisions[0];
        assert_eq!(custom["mode"].as_str().unwrap(), "user");
        assert!(
            custom["script"]
                .as_str()
                .unwrap()
                .contains("echo custom setup")
        );
    }

    #[test]
    fn test_generate_lima_config_extra_mounts() {
        let mounts = vec![
            Mount::rw(PathBuf::from("/tmp/project")),
            // Simulate an extra mount: read-only with different guest path
            Mount {
                host_path: PathBuf::from("/tmp/notes"),
                guest_path: PathBuf::from("/mnt/notes"),
                read_only: true,
            },
        ];

        let sandbox_config = SandboxConfig::default();
        let yaml = generate_lima_config("test-vm", &mounts, &sandbox_config).unwrap();

        let parsed: serde_yaml::Value = serde_yaml::from_str(&yaml).unwrap();
        let mount_list = parsed["mounts"].as_sequence().unwrap();
        assert_eq!(mount_list.len(), 2);

        // First mount: read-write, same host/guest
        let m0 = &mount_list[0];
        assert_eq!(m0["location"].as_str().unwrap(), "/tmp/project");
        assert_eq!(m0["writable"].as_bool().unwrap(), true);
        assert!(m0["mountPoint"].is_null());

        // Second mount: read-only, different guest path
        let m1 = &mount_list[1];
        assert_eq!(m1["location"].as_str().unwrap(), "/tmp/notes");
        assert_eq!(m1["writable"].as_bool().unwrap(), false);
        assert_eq!(m1["mountPoint"].as_str().unwrap(), "/mnt/notes");
    }
}
