use super::*;

#[test]
fn support_probe_reports_windows_appcontainer_or_unsupported_elsewhere() {
    let support = native_backend_sandbox_support();

    #[cfg(windows)]
    {
        assert!(!support.supports_sandboxed_execution());
        assert!(matches!(
            support.filesystem,
            SandboxFeatureSupport::Unsupported { reason } if reason.contains("not yet proven")
        ));
        assert!(matches!(
            support.network,
            SandboxFeatureSupport::Unsupported { reason } if reason.contains("not yet proven")
        ));
        assert!(matches!(
            support.process_cleanup,
            ProcessTreeCleanupSupport::WindowsJobObjectKillOnClose
        ));
    }

    #[cfg(unix)]
    {
        assert!(!support.supports_sandboxed_execution());
        assert!(matches!(
            support.filesystem,
            SandboxFeatureSupport::Unsupported { reason } if reason.contains("read-only payload")
        ));
        assert!(matches!(
            support.network,
            SandboxFeatureSupport::Unsupported { reason } if reason.contains("deny-by-default network")
        ));
        assert!(matches!(
            support.process_cleanup,
            ProcessTreeCleanupSupport::UnixProcessGroupKill
        ));
    }

    #[cfg(not(any(unix, windows)))]
    {
        assert!(!support.supports_sandboxed_execution());
        assert!(matches!(
            support.filesystem,
            SandboxFeatureSupport::Unsupported { reason } if reason.contains("read-only payload")
        ));
        assert!(matches!(
            support.network,
            SandboxFeatureSupport::Unsupported { reason } if reason.contains("deny-by-default network")
        ));
        assert!(matches!(
            support.process_cleanup,
            ProcessTreeCleanupSupport::Unsupported
        ));
    }
}

#[test]
fn sandboxed_policy_fails_closed_before_spawn() {
    let policy = NativeBackendLaunchPolicy::Sandboxed(
        NativeBackendSandboxRequest::deny_by_default_filesystem_and_network(),
    );
    let working_dir = std::env::temp_dir();

    let error = match StdioTransport::spawn_with_policy(
        "this-command-should-not-run",
        &[],
        Some(working_dir.as_path()),
        &[],
        policy,
    ) {
        Ok(_) => panic!("sandboxed launch should fail closed"),
        Err(error) => error,
    };

    assert!(matches!(
        error,
        McpError::Transport(message)
            if message.contains("this launch path is unsandboxed")
                && message.contains("filesystem=")
                && message.contains("network=")
    ));
}
