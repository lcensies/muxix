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

    // Use minimal Debian 12 image (aarch64 for Apple Silicon, x86_64 for Intel)
    // Debian genericcloud images are ~330MB vs Ubuntu's ~600MB
    let arch = std::env::consts::ARCH;
    let (image_url, image_arch) = if arch == "aarch64" || arch == "arm64" {
        (
            "https://cloud.debian.org/images/cloud/bookworm/latest/debian-12-genericcloud-arm64.qcow2",
            "aarch64",
        )
    } else {
        (
            "https://cloud.debian.org/images/cloud/bookworm/latest/debian-12-genericcloud-amd64.qcow2",
            "x86_64",
        )
    };

    let mut image_config = serde_yaml::Mapping::new();
    image_config.insert("location".into(), image_url.into());
    image_config.insert("arch".into(), image_arch.into());

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
    let system_script = r#"#!/bin/bash
set -eux
apt-get update
apt-get install -y --no-install-recommends curl ca-certificates git
"#;

    let user_script = r#"#!/bin/bash
set -eux
curl -fsSL https://claude.ai/install.sh | bash
curl -fsSL https://raw.githubusercontent.com/raine/workmux/main/scripts/install.sh | bash
# Ensure ~/.local/bin is on PATH for non-interactive shells
echo 'export PATH="$HOME/.local/bin:$PATH"' >> ~/.profile

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

    let mut provisions = vec![
        Value::Mapping(system_provision),
        Value::Mapping(user_provision),
    ];

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

        // User provision installs Claude Code and workmux
        assert!(yaml.contains("mode: user"));
        assert!(yaml.contains("claude.ai/install.sh"));
        assert!(yaml.contains("workmux/main/scripts/install.sh"));
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
}
