use std::ffi::{CStr, CString};
use std::path::PathBuf;

use ed25519_dalek::SigningKey;
use gbn_bridge_mobile_ffi::{
    gbn_mobile_runtime_call, gbn_mobile_runtime_close, gbn_mobile_runtime_create,
    gbn_mobile_string_free, BuildSyntheticUploadRequest, CreatorRuntimeConfig,
    HostCreatorBootstrapEndpoint, HostCreatorDhtSeed, HostCreatorDhtSeedImportRequest,
    HostCreatorReachability, MobileCreatorRuntime, TraceEventFilter,
};
use gbn_bridge_protocol::{
    publisher_identity, CreatorDhtEntry, CreatorDhtEntryUnsigned, SelfOnboardingState,
};
use serde_json::{json, Value};

fn temp_root(name: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!(
        "gbn-mobile-ffi-{name}-{}-{}",
        std::process::id(),
        now_ms()
    ));
    std::fs::create_dir_all(&path).unwrap();
    path
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

fn config(root: &std::path::Path, actor: &str) -> CreatorRuntimeConfig {
    CreatorRuntimeConfig {
        state_dir: root.join("state").to_string_lossy().to_string(),
        app_root_dir: Some(root.to_string_lossy().to_string()),
        publisher_public_key_hex: None,
        creator_id: Some(actor.to_string()),
        network_profile: "offline_test".to_string(),
        endpoint_config_json: None,
        log_level: "info".to_string(),
        evidence_dir: None,
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn host_seed(expires_at_ms: u64) -> String {
    let publisher_key = SigningKey::from_bytes(&[9_u8; 32]);
    let host_key = SigningKey::from_bytes(&[12_u8; 32]);
    let host_pub = publisher_identity(&host_key);
    let host_entry = CreatorDhtEntry::sign(
        CreatorDhtEntryUnsigned {
            node_id: "host-creator".to_string(),
            ip_addr: "198.51.100.10".to_string(),
            pub_key: host_pub.clone(),
            udp_punch_port: 4443,
            entry_expiry_ms: expires_at_ms,
        },
        &publisher_key,
        true,
    )
    .unwrap();
    serde_json::to_string(&HostCreatorDhtSeed {
        schema_version: 1,
        chain_id: "mobile-seed-chain".to_string(),
        run_id: "phase2-test-run".to_string(),
        host_creator_id: "host-creator".to_string(),
        host_creator_public_key_hex: hex(&host_pub.0),
        host_creator_entry: host_entry,
        host_creator_reachability: HostCreatorReachability {
            reachability_class: "direct".to_string(),
            capabilities: vec!["bootstrap_seed".to_string()],
        },
        host_creator_bootstrap_endpoints: vec![HostCreatorBootstrapEndpoint {
            url: Some("https://host-creator.example.test/bootstrap".to_string()),
            host: Some("198.51.100.10".to_string()),
            port: Some(443),
            tls_sni: Some("host-creator.example.test".to_string()),
            certificate_sha256: Some("abc123".to_string()),
        }],
        issued_at_ms: now_ms(),
        expires_at_ms,
        payload_hash: None,
        signature: Some("operator-signature-placeholder".to_string()),
        extra: Default::default(),
    })
    .unwrap()
}

#[test]
fn config_validation_rejects_missing_or_escaping_state_dir() {
    let root = temp_root("config");
    let mut missing = config(&root, "mobile-config");
    missing.state_dir.clear();
    assert!(MobileCreatorRuntime::new(missing).is_err());

    let mut escaping = config(&root, "mobile-config");
    escaping.state_dir = root
        .join("..")
        .join("outside")
        .to_string_lossy()
        .to_string();
    let error = match MobileCreatorRuntime::new(escaping) {
        Ok(_) => panic!("escaping state_dir should be rejected"),
        Err(error) => error,
    };
    assert_eq!(error.code(), "state_path_escape");
}

#[test]
fn startup_creates_identity_local_dht_and_events_then_restarts_same_identity() {
    let root = temp_root("startup");
    let runtime = MobileCreatorRuntime::new(config(&root, "mobile-startup")).unwrap();
    let metadata = runtime.node_metadata();
    assert_eq!(metadata.creator_id, "mobile-startup");
    assert!(root.join("state/identity.json").exists());
    assert!(root.join("state/local_dht.json").exists());
    assert!(root.join("state/evidence/events.jsonl").exists());
    drop(runtime);

    let restarted = MobileCreatorRuntime::new(config(&root, "mobile-startup")).unwrap();
    assert_eq!(
        restarted.node_metadata().identity_public_key_hex,
        metadata.identity_public_key_hex
    );
}

#[test]
fn qr_preview_and_import_persist_host_seed_without_onboarding() {
    let root = temp_root("seed");
    let runtime = MobileCreatorRuntime::new(config(&root, "mobile-seed")).unwrap();
    let payload = host_seed(now_ms() + 60_000);
    let preview = runtime.preview_bootstrap_dht_qr(&payload).unwrap();
    assert_eq!(preview.host_creator_id, "host-creator");

    let imported = runtime
        .import_host_creator_dht_seed(HostCreatorDhtSeedImportRequest { payload })
        .unwrap();
    assert_eq!(
        imported.self_onboarding_state,
        SelfOnboardingState::NewCreatorSeeded
    );
    let dht = runtime.local_dht();
    assert_eq!(dht.host_creator_entry.unwrap().node_id, "host-creator");
    assert_eq!(
        dht.self_onboarding_state,
        SelfOnboardingState::NewCreatorSeeded
    );
}

#[test]
fn qr_import_rejects_shortcuts_expired_and_private_admin_endpoints() {
    let root = temp_root("seed-reject");
    let runtime = MobileCreatorRuntime::new(config(&root, "mobile-seed-reject")).unwrap();
    assert!(runtime
        .preview_bootstrap_dht_qr(&host_seed(now_ms().saturating_sub(1)))
        .is_err());

    let mut value: Value = serde_json::from_str(&host_seed(now_ms() + 60_000)).unwrap();
    value["publisher_entry"] = json!({"node_id": "publisher"});
    assert!(runtime
        .preview_bootstrap_dht_qr(&serde_json::to_string(&value).unwrap())
        .is_err());

    let mut value: Value = serde_json::from_str(&host_seed(now_ms() + 60_000)).unwrap();
    value["host_creator_bootstrap_endpoints"][0]["url"] =
        json!("http://127.0.0.1:9090/v1/admin/host-join-relay");
    value["host_creator_bootstrap_endpoints"][0]["host"] = json!("127.0.0.1");
    assert!(runtime
        .preview_bootstrap_dht_qr(&serde_json::to_string(&value).unwrap())
        .is_err());
}

#[test]
fn synthetic_upload_trace_filter_and_evidence_export_work() {
    let root = temp_root("evidence");
    let runtime = MobileCreatorRuntime::new(config(&root, "mobile-evidence")).unwrap();
    let summary = runtime
        .build_synthetic_upload_session(BuildSyntheticUploadRequest {
            chain_id: Some("mobile-upload-chain".to_string()),
            size_bytes: 512,
            chunk_size: 128,
            sanitization_profile: "phase2-test".to_string(),
        })
        .unwrap();
    assert_eq!(summary.chain_id, "mobile-upload-chain");
    assert_eq!(summary.ciphertext_chunk_count, 4);

    let filtered = runtime
        .trace_events(TraceEventFilter {
            chain_id: Some("mobile-upload-chain".to_string()),
            event: None,
            operation: None,
            since_ms: None,
            until_ms: None,
            limit: None,
        })
        .unwrap();
    assert!(filtered
        .iter()
        .any(|event| event.event == "creator_upload_session_built"));

    let bundle = runtime.export_evidence().unwrap();
    assert!(bundle.files.iter().any(|file| file.path == "manifest.json"));
    assert!(bundle
        .remote_trace_queries
        .iter()
        .any(|query| query.chain_id == "mobile-upload-chain"));
    let identity_exported = bundle
        .files
        .iter()
        .any(|file| file.path.contains("identity"));
    assert!(!identity_exported);
}

#[test]
fn reset_clears_local_state_and_preserves_event_export_path() {
    let root = temp_root("reset");
    let runtime = MobileCreatorRuntime::new(config(&root, "mobile-reset")).unwrap();
    runtime
        .build_synthetic_upload_session(BuildSyntheticUploadRequest {
            chain_id: Some("mobile-reset-upload".to_string()),
            size_bytes: 32,
            chunk_size: 32,
            sanitization_profile: "phase2-test".to_string(),
        })
        .unwrap();
    let result = runtime
        .reset_state("mobile-reset-chain".to_string())
        .unwrap();
    assert_eq!(result.chain_id, "mobile-reset-chain");
    assert!(runtime.local_dht().publisher_entry.is_none());
    assert!(root.join("state/evidence/events.jsonl").exists());
}

#[test]
fn ffi_json_boundary_hides_native_pointer_and_catches_errors() {
    let root = temp_root("ffi");
    let config_json =
        CString::new(serde_json::to_string(&config(&root, "mobile-ffi")).unwrap()).unwrap();
    let response = unsafe { take_string(gbn_mobile_runtime_create(config_json.as_ptr())) };
    let parsed: Value = serde_json::from_str(&response).unwrap();
    assert_eq!(parsed["ok"], true);
    let handle = parsed["body"]["handle"].as_u64().unwrap();
    assert!(handle > 0);

    let method = CString::new("nodeMetadata").unwrap();
    let request = CString::new("{}").unwrap();
    let response = unsafe {
        take_string(gbn_mobile_runtime_call(
            handle,
            method.as_ptr(),
            request.as_ptr(),
        ))
    };
    let parsed: Value = serde_json::from_str(&response).unwrap();
    assert_eq!(parsed["ok"], true);
    assert_eq!(parsed["body"]["creator_id"], "mobile-ffi");

    let method = CString::new("sendDummy").unwrap();
    let response = unsafe {
        take_string(gbn_mobile_runtime_call(
            handle,
            method.as_ptr(),
            request.as_ptr(),
        ))
    };
    let parsed: Value = serde_json::from_str(&response).unwrap();
    assert_eq!(parsed["ok"], false);
    assert_eq!(parsed["error"]["code"], "not_implemented");

    let response = unsafe { take_string(gbn_mobile_runtime_close(handle)) };
    let parsed: Value = serde_json::from_str(&response).unwrap();
    assert_eq!(parsed["body"]["closed"], true);
}

unsafe fn take_string(ptr: *mut std::os::raw::c_char) -> String {
    assert!(!ptr.is_null());
    let value = CStr::from_ptr(ptr).to_string_lossy().to_string();
    gbn_mobile_string_free(ptr);
    value
}
