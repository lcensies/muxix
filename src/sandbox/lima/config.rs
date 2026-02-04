//! Lima configuration YAML generation.

use anyhow::Result;
use serde_yaml::Value;

use super::mounts::Mount;

/// Generate Lima configuration YAML.
pub fn generate_lima_config(_instance_name: &str, mounts: &[Mount]) -> Result<String> {
    let mut config = serde_yaml::Mapping::new();

    // Use minimal Ubuntu image (aarch64 for Apple Silicon, x86_64 for Intel)
    let arch = std::env::consts::ARCH;
    let (image_url, image_arch) = if arch == "aarch64" || arch == "arm64" {
        (
            "https://cloud-images.ubuntu.com/releases/25.10/release/ubuntu-25.10-server-cloudimg-arm64.img",
            "aarch64",
        )
    } else {
        (
            "https://cloud-images.ubuntu.com/releases/25.10/release/ubuntu-25.10-server-cloudimg-amd64.img",
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
    config.insert("cpus".into(), Value::Number(2.into()));
    config.insert("memory".into(), "2GiB".into());

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

        let yaml = generate_lima_config("test-vm", &mounts).unwrap();

        // Basic sanity checks
        assert!(yaml.contains("images:"));
        assert!(yaml.contains("mounts:"));
        assert!(yaml.contains("/Users/test/code"));
        assert!(yaml.contains("containerd:"));
    }
}
