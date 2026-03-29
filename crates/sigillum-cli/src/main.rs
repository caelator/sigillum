//! sigillum command-line interface for hardware-backed secret management.
//!
//! This module provides the CLI entry point and command handlers for the sigillum
//! vault system. It includes:
//! - Setup wizards for vault initialization
//! - Vault unlock/lock operations (passphrase and FIDO2)
//! - Secret CRUD operations (Tier 1 plaintext, Tier 2 encrypted)
//! - Compartment management and switching
//! - FIDO2 hardware key registration, removal, and status
//! - Encrypted snapshot backup and restore
//! - Daemon launcher for persistent unlock state
//!
//! All operations maintain strict security boundaries: encrypted secrets require
//! an unlocked compartment, and certain operations (FIDO2 registration/removal,
//! snapshot restore) demand all compartments be unlocked. The daemon provides
//! persistent management and Web UI control.

mod daemon_api;

use std::io::{self, Write};
use std::path::PathBuf;
use std::process;

use rand::RngCore;
use rand::rngs::OsRng;
use secrecy::ExposeSecret;
use sigillum_core::utils::{
    derive_key_from_passphrase, derive_key_with_salt, load_wrapped_master_key, save_salt,
    save_wrapped_master_key,
};
use sigillum_core::{FileVault, SecretStore, VaultConfig, VaultLifecycle};
use sigillum_core::{export_encrypted_snapshot, restore_encrypted_snapshot};
use sigillum_fido2::Fido2Manager;
use sigillum_fido2::config::{CompartmentMeta, SHARD_SLOTS};
use zeroize::Zeroizing;

fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 2 {
        print_usage();
        process::exit(1);
    }

    match args[1].as_str() {
        "setup" => cmd_setup(),
        "status" => cmd_status(),
        "unlock" => cmd_unlock(),
        "lock" => cmd_lock(),
        "set" => cmd_set(&args[2..]),
        "get" => cmd_get(&args[2..]),
        "delete" => cmd_delete(&args[2..]),
        "list" => cmd_list(),
        "set-api" => cmd_set_api(&args[2..]),
        "get-api" => cmd_get_api(&args[2..]),
        "delete-api" => cmd_delete_api(&args[2..]),
        "push" => cmd_push(&args[2..]),
        "fido2" => cmd_fido2(&args[2..]),
        "compartment" => cmd_compartment(&args[2..]),
        "backup" => cmd_backup(&args[2..]),
        "daemon" => cmd_daemon(&args[2..]),
        "api" => daemon_api::cmd_api(&args[2..]),
        "help" | "--help" | "-h" => print_usage(),
        "version" | "--version" | "-V" => println!("sigillum {}", env!("CARGO_PKG_VERSION")),
        other => {
            eprintln!("Unknown command: {other}");
            eprintln!("Run 'sigillum help' for usage.");
            process::exit(1);
        }
    }
}

fn print_usage() {
    println!(
        "\
sigillum — hardware-backed secret management

USAGE:
    sigillum <COMMAND> [ARGS]

COMMANDS:
    setup             First-time setup wizard
    status            Show vault status
    unlock            Unlock vault (auto-detects method)
    lock              Lock all compartments

    set <KEY>         Store a Tier 2 secret (encrypted, requires unlock)
    get <KEY>         Retrieve a Tier 2 secret
    delete <KEY>      Delete a Tier 2 secret
    list              List all keys (both tiers)

    set-api <KEY>     Store a Tier 1 API key (plaintext)
    get-api <KEY>     Retrieve a Tier 1 API key
    delete-api <KEY>  Delete a Tier 1 API key

    push              Copy secret between compartments:
      --from <N> --to <N> --key <K> [--as <NEW_NAME>] [--tier <1|2>]

    compartment <CMD> Compartment management:
      list                   List unlocked compartments
      switch --id <N>        Switch active compartment

    backup <CMD>      Encrypted snapshot management:
      export --output <PATH>  Write a passphrase-encrypted snapshot
      restore --input <PATH>  Restore from a passphrase-encrypted snapshot

    fido2 <CMD>       FIDO2 hardware key management:
      register --label <L>   Register a new hardware key
               --poison      Register as poison (duress) key
               --skip <L,..> Skip these keys during re-split
      list                   List registered keys
      remove --label <L>     Remove a hardware key
             --skip <L,..>   Skip these keys during re-split
      status                 Show FIDO2 status
      unlock --taps <N>      Unlock via FIDO2 (cascading)

    daemon [--port N] Start HTTP daemon (default: localhost:9743)
    api <CMD>         Talk to the local daemon API with JSON output

    version           Show version
    help              Show this message"
    );
}

// ── Discovery and Validation Helpers ────────────────────────────

/// Discover all currently unlocked compartments by probing vault state.
///
/// Scans all [`SHARD_SLOTS`] directories, extracting the master key and
/// compartment metadata from each unlocked vault. Returns an empty vec
/// if nothing is unlocked — callers decide whether that is fatal.
///
/// Each tuple contains `(CompartmentMeta, Zeroizing<master_key>)`.
fn discover_unlocked_compartments() -> Vec<(CompartmentMeta, Zeroizing<[u8; 32]>)> {
    let base = base_dir();
    let mut results = Vec::new();
    for i in 0..SHARD_SLOTS {
        let vault = vault_for_compartment(i);
        if vault.is_unlocked() {
            if let Some(mk) = vault.extract_master_key() {
                if let Ok(meta) = Fido2Manager::load_compartment_meta(&base, i, &mk) {
                    results.push((meta, mk));
                }
            }
        }
    }
    results
}

/// Discover unlocked compartments, exiting if none are available.
///
/// Convenience wrapper around [`discover_unlocked_compartments`] that
/// terminates the program if no compartments are unlocked. Use this for
/// operations that strictly require at least one active compartment.
fn require_unlocked_compartments() -> Vec<(CompartmentMeta, Zeroizing<[u8; 32]>)> {
    let compartments = discover_unlocked_compartments();
    if compartments.is_empty() {
        eprintln!("All compartments must be unlocked for this operation.");
        eprintln!(
            "Use the daemon for persistent unlock state, or unlock via FIDO2/passphrase first."
        );
        process::exit(1);
    }
    compartments
}

// ── Setup ────────────────────────────────────────────────────────

/// Set up a new vault with wizard prompts for compartment(s) and security tier.
fn cmd_setup() {
    let base = base_dir();
    let initialized_marker = base.join(".initialized");

    if initialized_marker.exists() {
        eprintln!("Vault already configured.");
        process::exit(1);
    }

    println!("=== SIGILLUM VAULT ===");
    println!();
    println!("Hardware-backed secret management with compartment isolation.");
    println!();
    println!("Choose a security tier:");
    println!();
    println!("  1) Simple");
    println!("     1 compartment, 1+ hardware keys. For daily use.");
    println!();
    println!("  2) More Secure");
    println!("     2 compartments (daily + secure). 2+ keys.");
    println!("     Tapping 2 keys unlocks both compartments.");
    println!();
    println!("  3) Legacy / Estate Planning");
    println!("     3 compartments (hot + cold + legacy). 3-of-5 quorum.");
    println!("     Tapping 3 keys unlocks all three.");
    println!();
    println!("  4) Custom");
    println!("     Define your own compartments and thresholds.");
    println!();
    println!("  5) Passphrase Only");
    println!("     Single compartment, no hardware key required.");
    println!();
    eprint!("Choice [2]: ");
    io::stderr().flush().unwrap();
    let mut choice = String::new();
    io::stdin().read_line(&mut choice).unwrap();
    let choice = choice.trim();
    let choice = if choice.is_empty() { "2" } else { choice };

    match choice {
        "1" => setup_fido2_preset(&[("daily", 1)]),
        "2" => setup_fido2_preset(&[("daily", 1), ("secure", 2)]),
        "3" => setup_fido2_preset(&[("hot", 1), ("cold", 2), ("legacy", 3)]),
        "4" => setup_fido2_custom(),
        "5" => setup_passphrase(),
        _ => {
            eprintln!("Invalid choice.");
            process::exit(1);
        }
    }
}

fn setup_passphrase() {
    let base = base_dir();
    let mgr = fido2_manager();

    eprint!("Compartment label [default]: ");
    io::stderr().flush().unwrap();
    let mut label = String::new();
    io::stdin().read_line(&mut label).unwrap();
    let label = label.trim();
    let label = if label.is_empty() { "default" } else { label };

    let passphrase = prompt_passphrase_confirm();

    // Generate random master key
    let mut master_key = [0u8; 32];
    OsRng.fill_bytes(&mut master_key);

    // Create compartment metadata
    let meta = CompartmentMeta {
        id: 0,
        label: label.to_string(),
        threshold: 1,
        passphrase_mode: Some("wrapped".into()),
    };

    // Initialize vault
    let vault = vault_for_compartment(0);
    if let Err(e) = vault.initialize(&master_key) {
        eprintln!("Failed to initialize vault: {e}");
        process::exit(1);
    }

    // Save encrypted compartment metadata
    Fido2Manager::save_compartment_meta(&base, &meta, &master_key).unwrap_or_else(|e| {
        eprintln!("Failed to save compartment meta: {e}");
        process::exit(1);
    });

    // Wrap master key with passphrase
    let (wrap_key, salt) = derive_key_from_passphrase(&passphrase);
    if let Err(e) = save_salt(&salt, &compartment_salt_path(0)) {
        eprintln!("Failed to save salt: {e}");
        process::exit(1);
    }
    if let Err(e) =
        save_wrapped_master_key(&master_key, &wrap_key, &compartment_wrapped_key_path(0))
    {
        eprintln!("Failed to save wrapped key: {e}");
        process::exit(1);
    }

    // Setup dummy directories for deniability
    Fido2Manager::setup_dummy_directories(&base, &[0]).unwrap_or_else(|e| {
        eprintln!("Failed to setup dummy directories: {e}");
        process::exit(1);
    });

    // Save empty fido2 config (setup_dummy_directories already wrote .initialized)
    if let Err(e) = mgr.save_config_raw(&sigillum_fido2::config::Fido2Config {
        total_shares: 1,
        keys: Vec::new(),
    }) {
        eprintln!("Warning: failed to save FIDO2 config: {e}");
    }

    zeroize::Zeroize::zeroize(&mut master_key);

    println!();
    println!("Vault initialized. Compartment \"{label}\" created.");
    println!("Remember your passphrase — it cannot be recovered.");
}

fn setup_fido2_preset(presets: &[(&str, usize)]) {
    let base = base_dir();
    let mgr = fido2_manager();

    let device_count = sigillum_fido2::hid::detect_devices();
    if device_count == 0 {
        eprintln!("No FIDO2 device detected. Insert your hardware key and try again.");
        process::exit(1);
    }
    println!("Detected {device_count} FIDO2 device(s).");
    println!();

    let metas: Vec<CompartmentMeta> = presets
        .iter()
        .map(|(label, threshold)| CompartmentMeta {
            id: *threshold - 1, // 0-indexed
            label: label.to_string(),
            threshold: *threshold,
            passphrase_mode: None,
        })
        .collect();

    println!("Compartments to create:");
    for m in &metas {
        println!(
            "  [{}] {} — {} tap{}",
            m.id,
            m.label,
            m.threshold,
            if m.threshold > 1 { "s" } else { "" }
        );
    }
    println!();

    let pin = prompt_optional_pin();
    eprint!("Key label (e.g. 'yubikey-primary'): ");
    io::stderr().flush().unwrap();
    let mut label = String::new();
    io::stdin().read_line(&mut label).unwrap();
    let label = label.trim();

    if label.is_empty() {
        eprintln!("Label cannot be empty.");
        process::exit(1);
    }

    // Generate master keys per compartment
    let mut master_keys: Vec<(CompartmentMeta, [u8; 32])> = Vec::new();
    for m in &metas {
        let mut mk = [0u8; 32];
        OsRng.fill_bytes(&mut mk);
        master_keys.push((m.clone(), mk));
    }

    // Build refs for registration
    let mk_refs: Vec<(CompartmentMeta, &[u8; 32])> =
        master_keys.iter().map(|(m, mk)| (m.clone(), mk)).collect();

    println!();
    println!("Touch your FIDO2 key now...");

    match mgr.register_key(pin.as_deref(), label, &mk_refs, &[]) {
        Ok(_result) => {
            // Initialize each compartment vault
            for (meta, mk) in &master_keys {
                let vault = vault_for_compartment(meta.id);
                if let Err(e) = vault.initialize(mk) {
                    eprintln!("Failed to initialize compartment {}: {e}", meta.id);
                    process::exit(1);
                }
                // Save encrypted meta
                Fido2Manager::save_compartment_meta(&base, meta, mk).unwrap_or_else(|e| {
                    eprintln!("Failed to save meta for compartment {}: {e}", meta.id);
                    process::exit(1);
                });
            }

            let real_ids: Vec<usize> = metas.iter().map(|m| m.id).collect();

            // Setup dummy directories
            Fido2Manager::setup_dummy_directories(&base, &real_ids).unwrap_or_else(|e| {
                eprintln!("Failed to setup dummy directories: {e}");
                process::exit(1);
            });

            // setup_dummy_directories already wrote .initialized and created dirs

            println!();
            println!(
                "FIDO2 key '{label}' registered. {} compartment(s) created.",
                metas.len()
            );

            // Ask about passphrase fallback
            eprint!("Set a fallback passphrase for all compartments? [y/N]: ");
            io::stderr().flush().unwrap();
            let mut answer = String::new();
            io::stdin().read_line(&mut answer).unwrap();
            if answer.trim().eq_ignore_ascii_case("y") {
                let passphrase = prompt_passphrase_confirm();
                for (meta, mk) in &master_keys {
                    let (wrap_key, salt) = derive_key_from_passphrase(&passphrase);
                    if let Err(e) = save_salt(&salt, &compartment_salt_path(meta.id)) {
                        eprintln!("Failed to save salt for compartment {}: {e}", meta.id);
                        process::exit(1);
                    }
                    if let Err(e) = save_wrapped_master_key(
                        mk,
                        &wrap_key,
                        &compartment_wrapped_key_path(meta.id),
                    ) {
                        eprintln!(
                            "Failed to save wrapped key for compartment {}: {e}",
                            meta.id
                        );
                        process::exit(1);
                    }
                    // Update meta with passphrase_mode
                    let updated_meta = CompartmentMeta {
                        passphrase_mode: Some("wrapped".into()),
                        ..meta.clone()
                    };
                    if let Err(e) = Fido2Manager::save_compartment_meta(&base, &updated_meta, mk) {
                        eprintln!(
                            "Warning: failed to update meta for compartment {}: {e}",
                            meta.id
                        );
                    }
                }
                println!("Fallback passphrase configured for all compartments.");
            }

            // Ask about additional keys
            loop {
                eprint!("Register another hardware key? [y/N]: ");
                io::stderr().flush().unwrap();
                let mut answer = String::new();
                io::stdin().read_line(&mut answer).unwrap();
                if !answer.trim().eq_ignore_ascii_case("y") {
                    break;
                }

                let pin = prompt_optional_pin();
                eprint!("Key label: ");
                io::stderr().flush().unwrap();
                let mut next_label = String::new();
                io::stdin().read_line(&mut next_label).unwrap();
                let next_label = next_label.trim();

                println!("Touch your FIDO2 key now...");
                match mgr.register_key(pin.as_deref(), next_label, &mk_refs, &[]) {
                    Ok(r) => println!("Key '{next_label}' registered ({} total).", r.total_keys),
                    Err(e) => eprintln!("Failed: {e}"),
                }
            }

            // Zeroize master keys
            for (_, mk) in &mut master_keys {
                zeroize::Zeroize::zeroize(mk);
            }

            println!();
            println!("Setup complete.");
        }
        Err(e) => {
            for (_, mk) in &mut master_keys {
                zeroize::Zeroize::zeroize(mk);
            }
            eprintln!("FIDO2 registration failed: {e}");
            process::exit(1);
        }
    }
}

fn setup_fido2_custom() {
    println!();
    println!("Define compartments (each needs a unique tap-count threshold):");
    println!();

    let mut comps: Vec<(&str, usize)> = Vec::new();
    let mut owned_labels: Vec<String> = Vec::new();
    loop {
        eprint!("Compartment label (empty to finish): ");
        io::stderr().flush().unwrap();
        let mut label = String::new();
        io::stdin().read_line(&mut label).unwrap();
        let label = label.trim().to_string();
        if label.is_empty() {
            break;
        }

        eprint!("Threshold (tap count): ");
        io::stderr().flush().unwrap();
        let mut t = String::new();
        io::stdin().read_line(&mut t).unwrap();
        let threshold: usize = t.trim().parse().unwrap_or_else(|_| {
            eprintln!("Invalid threshold");
            process::exit(1);
        });

        owned_labels.push(label);
        // We'll build the presets after collecting all labels
        comps.push(("", threshold));
    }

    if comps.is_empty() {
        eprintln!("At least one compartment required.");
        process::exit(1);
    }

    // Build presets from owned labels
    let presets: Vec<(&str, usize)> = owned_labels
        .iter()
        .zip(comps.iter())
        .map(|(label, (_, t))| (label.as_str(), *t))
        .collect();

    setup_fido2_preset(&presets);
}

// ── Status (deniable) ──────────────────────────────────────────

/// Show vault status without revealing compartment details for deniability.
///
/// Reports lock status, FIDO2 enablement, and available devices, but does not
/// disclose the number of compartments, their thresholds, or any labels.
fn cmd_status() {
    let base = base_dir();
    let initialized = base.join(".initialized").exists();

    if !initialized {
        println!("NOT INITIALIZED");
        println!("Run 'sigillum setup' to create a vault.");
        return;
    }

    let mgr = fido2_manager();
    let config = mgr.load_config_raw().unwrap_or_else(|e| {
        eprintln!("Failed to load FIDO2 config: {e}");
        process::exit(1);
    });
    let has_fido = !config.keys.is_empty();
    let device_count = sigillum_fido2::hid::detect_devices();

    // DENIABILITY: no compartment count, no threshold info, no labels
    println!("=== SIGILLUM VAULT ===");
    println!("Status:          LOCKED");
    println!("FIDO2 enabled:   {has_fido}");
    println!("FIDO2 keys:      {}", config.keys.len());
    println!("Devices present: {device_count}");
    println!();
    println!("Run 'sigillum unlock' to unlock.");
}

// ── Unlock ──────────────────────────────────────────────────────

/// Unlock one or more compartments via passphrase or FIDO2.
///
/// Attempts passphrase unlock first (if enabled), then falls back to FIDO2.
/// Unlocked compartments remain accessible in memory until locked or the
/// process exits. The daemon provides persistent unlock state.
fn cmd_unlock() {
    let base = base_dir();
    if !base.join(".initialized").exists() {
        eprintln!("No vault configured. Run 'sigillum setup' first.");
        process::exit(1);
    }

    let mgr = fido2_manager();
    let config = mgr.load_config_raw().unwrap_or_else(|e| {
        eprintln!("Failed to load FIDO2 config: {e}");
        process::exit(1);
    });
    let has_fido = !config.keys.is_empty();
    let has_passphrase = has_any_passphrase_compartment();

    if has_fido && has_passphrase {
        if sigillum_fido2::hid::is_device_present() {
            println!("Choose unlock method:");
            println!("  1) Hardware key (FIDO2)");
            println!("  2) Passphrase");
            eprint!("Choice [1]: ");
            io::stderr().flush().unwrap();
            let mut choice = String::new();
            io::stdin().read_line(&mut choice).unwrap();
            if choice.trim() == "2" {
                unlock_passphrase();
            } else {
                unlock_fido2();
            }
        } else {
            println!("No FIDO2 device detected. Using passphrase.");
            unlock_passphrase();
        }
    } else if has_fido {
        unlock_fido2();
    } else if has_passphrase {
        unlock_passphrase();
    } else {
        eprintln!("No unlock method configured.");
        process::exit(1);
    }
}

fn unlock_passphrase() {
    let base = base_dir();
    let passphrase = prompt_passphrase();

    // DENIABILITY: probe all 100 compartment directories silently
    let mut unlocked_any = false;
    for i in 0..SHARD_SLOTS {
        let salt_path = compartment_salt_path(i);
        let wrapped_key_path = compartment_wrapped_key_path(i);

        let salt = match std::fs::read(&salt_path) {
            Ok(s) if s.len() == 32 => s,
            _ => continue,
        };

        let wrap_key = derive_key_with_salt(&passphrase, &salt);
        if let Some(master_key) = load_wrapped_master_key(&wrap_key, &wrapped_key_path) {
            // Try to decrypt meta.enc to discover compartment
            match Fido2Manager::load_compartment_meta(&base, i, &master_key) {
                Ok(meta) => {
                    let vault = vault_for_compartment(i);
                    vault.load_master_key(*master_key);
                    if vault.verify_master_key() {
                        println!("Unlocked compartment \"{}\" (id={}).", meta.label, meta.id);
                        unlocked_any = true;
                    } else {
                        vault.zeroize_master_key();
                    }
                }
                Err(_) => {
                    // Not a real compartment or wrong key — ignore silently
                }
            }
        }
    }

    if !unlocked_any {
        eprintln!("No compartment matched this passphrase.");
        process::exit(1);
    }
}

fn unlock_fido2() {
    let base = base_dir();
    let mgr = fido2_manager();

    // DENIABILITY: no compartment hints, just ask for tap count
    eprint!("Keys to tap: ");
    io::stderr().flush().unwrap();
    let mut taps_str = String::new();
    io::stdin().read_line(&mut taps_str).unwrap();
    let taps: usize = taps_str.trim().parse().unwrap_or_else(|_| {
        eprintln!("Invalid tap count");
        process::exit(1);
    });

    let pin = prompt_optional_pin();
    println!("Touch your FIDO2 key now...");
    let pins: Vec<String> = pin.into_iter().collect();

    match mgr.authenticate_cascading(&pins, taps, &base, None) {
        Ok(results) => {
            if results.is_empty() {
                eprintln!("No compartments could be unlocked.");
                process::exit(1);
            }

            println!();
            println!("Unlocked {} compartment(s):", results.len());
            for (meta, _mk) in &results {
                println!("  [{}] {}", meta.id, meta.label);
            }

            // Load master keys into vaults
            for (meta, mk) in &results {
                let vault = vault_for_compartment(meta.id);
                vault.load_master_key(**mk);
            }
        }
        Err(e) => {
            eprintln!("FIDO2 unlock failed: {e}");
            process::exit(1);
        }
    }
}

/// Lock all compartments (daemon only).
///
/// The CLI is stateless and does not maintain unlock state between commands.
/// To lock compartments, use the daemon's web UI or HTTP API.
fn cmd_lock() {
    eprintln!("CLI is stateless — keys are not held in memory between commands.");
    eprintln!("To lock the daemon, use the web UI or: curl -X POST http://localhost:9743/api/lock");
}

// ── Snapshot backup ─────────────────────────────────────────────

/// Export encrypted snapshots or restore from backups.
///
/// Exports all unlocked compartments to a passphrase-encrypted file, or
/// restores from a previously exported snapshot. All compartments must be
/// unlocked for restore to succeed.
fn cmd_backup(args: &[String]) {
    if args.is_empty() {
        eprintln!("Usage: sigillum backup <export|restore>");
        process::exit(1);
    }

    match args[0].as_str() {
        "export" => backup_export(&args[1..]),
        "restore" => backup_restore(&args[1..]),
        other => {
            eprintln!("Unknown backup command: {other}");
            process::exit(1);
        }
    }
}

fn backup_export(args: &[String]) {
    let output = parse_flag(args, "--output").unwrap_or_else(|| {
        eprintln!("Usage: sigillum backup export --output <PATH>");
        process::exit(1);
    });

    let passphrase = prompt_passphrase_confirm();
    let base = base_dir();
    let (snapshot, summary) =
        export_encrypted_snapshot(&base, passphrase.as_str()).unwrap_or_else(|e| {
            eprintln!("Snapshot export failed: {e}");
            process::exit(1);
        });

    let output = PathBuf::from(output);
    if let Some(parent) = output.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).unwrap_or_else(|e| {
                eprintln!("Failed to create output directory: {e}");
                process::exit(1);
            });
        }
    }
    std::fs::write(&output, snapshot).unwrap_or_else(|e| {
        eprintln!("Failed to write snapshot: {e}");
        process::exit(1);
    });

    println!(
        "Snapshot written to {} ({} files, {} bytes).",
        output.display(),
        summary.file_count,
        summary.total_bytes
    );
}

fn backup_restore(args: &[String]) {
    let input = parse_flag(args, "--input").unwrap_or_else(|| {
        eprintln!("Usage: sigillum backup restore --input <PATH>");
        process::exit(1);
    });

    let snapshot = std::fs::read(&input).unwrap_or_else(|e| {
        eprintln!("Failed to read snapshot: {e}");
        process::exit(1);
    });
    let passphrase = prompt_passphrase();
    let base = base_dir();

    let summary =
        restore_encrypted_snapshot(&base, passphrase.as_str(), &snapshot).unwrap_or_else(|e| {
            eprintln!("Snapshot restore failed: {e}");
            process::exit(1);
        });

    println!(
        "Snapshot restored from {} ({} files, {} bytes).",
        input, summary.file_count, summary.total_bytes
    );
    println!("The vault is restored on disk. Unlock it again before managing secrets.");
}

// ── Secrets (operate on first unlocked vault) ──────────────

/// Probe all compartment directories to find one with an unlocked vault.
/// In CLI mode this is limited since state doesn't persist between invocations.
/// The daemon is the preferred way to manage secrets persistently.
fn find_unlocked_vault() -> (usize, FileVault) {
    for i in 0..SHARD_SLOTS {
        let vault = vault_for_compartment(i);
        if vault.is_unlocked() {
            return (i, vault);
        }
    }
    eprintln!("No compartment is unlocked. Run 'sigillum unlock' first.");
    eprintln!("(Note: use the daemon for persistent unlock state.)");
    process::exit(1);
}

/// Store a Tier 2 encrypted secret in the first unlocked compartment.
fn cmd_set(args: &[String]) {
    let key = require_arg(args, "set", "<KEY>");
    let (_, vault) = find_unlocked_vault();
    let value = prompt_secret("Value: ");
    if let Err(e) = vault.set_secret(&key, &value) {
        eprintln!("Failed: {e}");
        process::exit(1);
    }
    println!("Secret '{key}' stored (Tier 2, encrypted).");
}

/// Retrieve a Tier 2 encrypted secret from the first unlocked compartment.
fn cmd_get(args: &[String]) {
    let key = require_arg(args, "get", "<KEY>");
    let (_, vault) = find_unlocked_vault();
    match vault.read_secret(&key) {
        Ok(Some(val)) => println!("{}", val.expose_secret()),
        Ok(None) => {
            eprintln!("Secret '{key}' not found.");
            process::exit(1);
        }
        Err(e) => {
            eprintln!("Failed to read secret '{key}': {e}");
            process::exit(1);
        }
    }
}

/// Delete a Tier 2 encrypted secret from the first unlocked compartment.
fn cmd_delete(args: &[String]) {
    let key = require_arg(args, "delete", "<KEY>");
    let (_, vault) = find_unlocked_vault();
    if let Err(e) = vault.delete_secret(&key) {
        eprintln!("Failed: {e}");
        process::exit(1);
    }
    println!("Secret '{key}' deleted.");
}

/// List all secrets (both Tier 1 API keys and Tier 2 encrypted) across unlocked compartments.
fn cmd_list() {
    let mut found_any = false;
    for i in 0..SHARD_SLOTS {
        let vault = vault_for_compartment(i);
        if vault.is_unlocked() {
            let api_keys = match vault.read_api_keys() {
                Ok(keys) => keys,
                Err(e) => {
                    eprintln!("Failed to list API keys for compartment {i}: {e}");
                    process::exit(1);
                }
            };
            if !api_keys.is_empty() {
                println!("=== Compartment {i}: Tier 1 (API Keys) ===");
                for k in &api_keys {
                    println!("  {k}");
                }
                found_any = true;
            }
            let secrets = match vault.read_secrets() {
                Ok(keys) => keys,
                Err(e) => {
                    eprintln!("Failed to list encrypted secrets for compartment {i}: {e}");
                    process::exit(1);
                }
            };
            if !secrets.is_empty() {
                println!("=== Compartment {i}: Tier 2 (Encrypted Secrets) ===");
                for k in &secrets {
                    println!("  {k}");
                }
                found_any = true;
            }
        }
    }
    if !found_any {
        println!("No keys found (unlock a compartment to see secrets).");
    }
}

/// Store a Tier 1 plaintext API key in the first unlocked compartment.
fn cmd_set_api(args: &[String]) {
    let key = require_arg(args, "set-api", "<KEY>");
    let (_, vault) = find_unlocked_vault();
    let value = prompt_secret("Value: ");
    if let Err(e) = vault.set_api_key(&key, &value) {
        eprintln!("Failed: {e}");
        process::exit(1);
    }
    println!("API key '{key}' stored (Tier 1).");
}

/// Retrieve a Tier 1 plaintext API key from the first unlocked compartment.
fn cmd_get_api(args: &[String]) {
    let key = require_arg(args, "get-api", "<KEY>");
    let (_, vault) = find_unlocked_vault();
    match vault.read_api_key(&key) {
        Ok(Some(val)) => println!("{}", val.expose_secret()),
        Ok(None) => {
            eprintln!("API key '{key}' not found.");
            process::exit(1);
        }
        Err(e) => {
            eprintln!("Failed to read API key '{key}': {e}");
            process::exit(1);
        }
    }
}

/// Delete a Tier 1 plaintext API key from the first unlocked compartment.
fn cmd_delete_api(args: &[String]) {
    let key = require_arg(args, "delete-api", "<KEY>");
    let (_, vault) = find_unlocked_vault();
    if let Err(e) = vault.delete_api_key(&key) {
        eprintln!("Failed: {e}");
        process::exit(1);
    }
    println!("API key '{key}' deleted.");
}

// ── Push-Down Command ───────────────────────────────────────────

/// Copy a secret between compartments (push from one to another).
///
/// Supports both Tier 1 API keys and Tier 2 encrypted secrets, with optional
/// renaming via `--as`. Both source and destination compartments must be unlocked.
fn cmd_push(args: &[String]) {
    let from_id: usize = parse_flag(args, "--from")
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| {
            eprintln!("Usage: sigillum push --from <N> --to <N> --key <K> [--as <NEW_NAME>] [--tier <1|2>]");
            process::exit(1);
        });
    let to_id: usize = parse_flag(args, "--to")
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| {
            eprintln!("Usage: sigillum push --from <N> --to <N> --key <K> [--as <NEW_NAME>] [--tier <1|2>]");
            process::exit(1);
        });
    let key = parse_flag(args, "--key").unwrap_or_else(|| {
        eprintln!(
            "Usage: sigillum push --from <N> --to <N> --key <K> [--as <NEW_NAME>] [--tier <1|2>]"
        );
        process::exit(1);
    });
    let new_key = parse_flag(args, "--as").unwrap_or_else(|| key.clone());
    let tier: u8 = parse_flag(args, "--tier")
        .and_then(|s| s.parse().ok())
        .unwrap_or(2);

    if from_id == to_id {
        eprintln!("Source and target compartments must differ.");
        process::exit(1);
    }

    let source_vault = vault_for_compartment(from_id);
    let target_vault = vault_for_compartment(to_id);

    if !source_vault.is_unlocked() {
        eprintln!("Source compartment {from_id} is not unlocked.");
        process::exit(1);
    }
    if !target_vault.is_unlocked() {
        eprintln!("Target compartment {to_id} is not unlocked.");
        process::exit(1);
    }

    match tier {
        1 => {
            // Push API key
            let value = match source_vault.read_api_key(&key) {
                Ok(Some(v)) => v,
                Ok(None) => {
                    eprintln!("API key '{key}' not found in compartment {from_id}.");
                    process::exit(1);
                }
                Err(e) => {
                    eprintln!("Failed to read API key '{key}' from compartment {from_id}: {e}");
                    process::exit(1);
                }
            };
            if let Err(e) = target_vault.set_api_key(&new_key, value.expose_secret()) {
                eprintln!("Failed to write to target: {e}");
                process::exit(1);
            }
        }
        2 => {
            // Push encrypted secret
            let value = match source_vault.read_secret(&key) {
                Ok(Some(v)) => v,
                Ok(None) => {
                    eprintln!("Secret '{key}' not found in compartment {from_id}.");
                    process::exit(1);
                }
                Err(e) => {
                    eprintln!("Failed to read secret '{key}' from compartment {from_id}: {e}");
                    process::exit(1);
                }
            };
            if let Err(e) = target_vault.set_secret(&new_key, value.expose_secret()) {
                eprintln!("Failed to write to target: {e}");
                process::exit(1);
            }
        }
        _ => {
            eprintln!("Invalid tier: {tier}. Use 1 (API key) or 2 (encrypted secret).");
            process::exit(1);
        }
    }

    println!(
        "Pushed '{key}' from compartment {from_id} to '{new_key}' in compartment {to_id} (tier {tier})."
    );
}

// ── Compartment subcommands ─────────────────────────────────────

/// Manage compartments: list unlocked ones or switch active compartment.
fn cmd_compartment(args: &[String]) {
    if args.is_empty() {
        eprintln!("Usage: sigillum compartment <list|switch>");
        process::exit(1);
    }

    match args[0].as_str() {
        "list" => compartment_list(),
        "switch" => compartment_switch(&args[1..]),
        other => {
            eprintln!("Unknown compartment command: {other}");
            process::exit(1);
        }
    }
}

fn compartment_list() {
    let base = base_dir();
    if !base.join(".initialized").exists() {
        println!("Not initialized. Run 'sigillum setup' first.");
        return;
    }

    // DENIABILITY: only show compartments we can discover
    // In CLI mode we can't discover anything without unlocking first
    println!("=== Unlocked Compartments ===");
    println!("(Use the daemon + web UI for persistent compartment management.)");

    let compartments = discover_unlocked_compartments();
    if compartments.is_empty() {
        println!("  No compartments currently unlocked.");
        println!("  Run 'sigillum unlock' first, or use the daemon.");
    } else {
        for (meta, _) in compartments {
            println!(
                "  [{}] {} (threshold={})",
                meta.id, meta.label, meta.threshold
            );
        }
    }
}

fn compartment_switch(args: &[String]) {
    let id: usize = parse_flag(args, "--id")
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| {
            eprintln!("Usage: sigillum compartment switch --id <N>");
            process::exit(1);
        });

    let vault = vault_for_compartment(id);
    if !vault.is_unlocked() {
        eprintln!("Compartment {id} is not unlocked.");
        process::exit(1);
    }

    // Write active compartment marker
    let base = base_dir();
    if let Err(e) = std::fs::write(base.join(".active_compartment"), id.to_string()) {
        eprintln!("Warning: failed to write active compartment marker: {e}");
    }
    println!("Switched active compartment to {id}.");
}

// ── FIDO2 subcommands ───────────────────────────────────────────

/// Manage FIDO2 hardware keys: register, remove, list, check status, or unlock.
fn cmd_fido2(args: &[String]) {
    if args.is_empty() {
        eprintln!("Usage: sigillum fido2 <register|list|remove|status|unlock>");
        process::exit(1);
    }

    let mgr = fido2_manager();

    match args[0].as_str() {
        "register" => fido2_register(&mgr, &args[1..]),
        "list" => fido2_list(&mgr),
        "remove" => fido2_remove(&mgr, &args[1..]),
        "status" => fido2_status(&mgr),
        "unlock" => unlock_fido2(),
        other => {
            eprintln!("Unknown fido2 command: {other}");
            process::exit(1);
        }
    }
}

fn fido2_register(mgr: &Fido2Manager, args: &[String]) {
    let label = parse_label_arg(args, "register");
    let is_poison = has_flag(args, "--poison");
    let skip_labels: Vec<String> = parse_flag(args, "--skip")
        .map(|s| s.split(',').map(|l| l.trim().to_string()).collect())
        .unwrap_or_default();

    if is_poison {
        // Poison key: generate random shard data, no master keys needed
        // Discover compartment metadata from unlocked vaults
        let compartments = discover_unlocked_compartments();
        let metas: Vec<CompartmentMeta> = compartments.into_iter().map(|(m, _)| m).collect();

        if metas.is_empty() {
            eprintln!("At least one compartment must be unlocked to read metadata.");
            process::exit(1);
        }

        let config = mgr.load_config_raw().unwrap_or_else(|e| {
            eprintln!("Failed to load FIDO2 config: {e}");
            process::exit(1);
        });
        let max_threshold = metas.iter().map(|m| m.threshold).max().unwrap_or(1);

        eprintln!("WARNING: Registering a POISON key.");
        eprintln!("  Current keys: {}", config.keys.len());
        eprintln!("  Max threshold: {max_threshold}");
        eprintln!("  Including this key during unlock will cause SILENT FAILURE.");
        eprintln!("  No data will be destroyed. Exclude it and retry with real keys.");
        eprint!("Continue? [y/N]: ");
        io::stderr().flush().unwrap();
        let mut answer = String::new();
        io::stdin().read_line(&mut answer).unwrap();
        if !answer.trim().eq_ignore_ascii_case("y") {
            eprintln!("Aborted.");
            process::exit(0);
        }

        let pin = prompt_optional_pin();
        println!("Touch your FIDO2 key now...");

        match mgr.register_key_poison(pin.as_deref(), &label, &metas) {
            Ok(total) => println!("Poison key '{label}' registered ({total} total)."),
            Err(e) => {
                eprintln!("Registration failed: {e}");
                process::exit(1);
            }
        }
        return;
    }

    // Normal key registration
    let metas_and_keys = require_unlocked_compartments();

    let mk_refs: Vec<(CompartmentMeta, &[u8; 32])> = metas_and_keys
        .iter()
        .map(|(meta, mk)| (meta.clone(), &**mk))
        .collect();

    let pin = prompt_optional_pin();
    println!("Touch your FIDO2 key now...");

    match mgr.register_key(pin.as_deref(), &label, &mk_refs, &skip_labels) {
        Ok(result) => println!("Key '{label}' registered ({} total).", result.total_keys),
        Err(e) => {
            eprintln!("Registration failed: {e}");
            process::exit(1);
        }
    }
}

fn fido2_list(mgr: &Fido2Manager) {
    let keys = mgr.list_keys().unwrap_or_else(|e| {
        eprintln!("Failed to load FIDO2 keys: {e}");
        process::exit(1);
    });
    if keys.is_empty() {
        println!("No FIDO2 keys registered.");
        return;
    }
    println!("=== Registered FIDO2 Keys ===");
    for k in &keys {
        println!(
            "  {} ({}...) — {}",
            k.label, k.credential_id_short, k.registered_at,
        );
    }
}

fn fido2_remove(mgr: &Fido2Manager, args: &[String]) {
    let label = parse_label_arg(args, "remove");
    let skip_labels: Vec<String> = parse_flag(args, "--skip")
        .map(|s| s.split(',').map(|l| l.trim().to_string()).collect())
        .unwrap_or_default();

    // Need master keys for all unlocked compartments
    let metas_and_keys = require_unlocked_compartments();

    let mk_refs: Vec<(CompartmentMeta, &[u8; 32])> = metas_and_keys
        .iter()
        .map(|(meta, mk)| (meta.clone(), &**mk))
        .collect();

    let pin = prompt_optional_pin();
    println!("Tap remaining keys to re-encrypt shards...");

    match mgr.remove_key(&label, &mk_refs, pin.as_deref(), &skip_labels) {
        Ok(()) => println!("Key '{label}' removed."),
        Err(e) => {
            eprintln!("Removal failed: {e}");
            process::exit(1);
        }
    }
}

fn fido2_status(mgr: &Fido2Manager) {
    let s = mgr.status().unwrap_or_else(|e| {
        eprintln!("Failed to load FIDO2 status: {e}");
        process::exit(1);
    });
    let device_count = sigillum_fido2::hid::detect_devices();

    println!("=== FIDO2 STATUS ===");
    println!("Enabled:         {}", s.enabled);
    println!("Registered keys: {}", s.key_count);
    println!("Devices present: {device_count}");
}

// ── Daemon ──────────────────────────────────────────────────────

/// Start the HTTP daemon for persistent unlock state and web UI management.
///
/// Listens on localhost (default port 9743) and provides REST/WebSocket APIs
/// for compartment management, secret CRUD, FIDO2 operations, and unlock persistence.
fn cmd_daemon(args: &[String]) {
    let mut port: u16 = 9743;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--port" | "-p" => {
                i += 1;
                port = args.get(i).and_then(|s| s.parse().ok()).unwrap_or_else(|| {
                    eprintln!("Invalid port number.");
                    process::exit(1);
                });
            }
            _ => {
                eprintln!("Unknown daemon flag: {}", args[i]);
                process::exit(1);
            }
        }
        i += 1;
    }

    let addr: std::net::SocketAddr = ([127, 0, 0, 1], port).into();
    let base_dir = base_dir();

    let rt = tokio::runtime::Runtime::new().unwrap_or_else(|e| {
        eprintln!("Failed to start async runtime: {e}");
        process::exit(1);
    });

    if let Err(e) = rt.block_on(sigillum_daemon::run(addr, base_dir)) {
        eprintln!("Daemon error: {e}");
        process::exit(1);
    }
}

// ── Helpers ─────────────────────────────────────────────────────

fn base_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".sigillum")
}

fn fido2_manager() -> Fido2Manager {
    Fido2Manager::new(base_dir().join("fido2_keys.json"))
}

fn vault_for_compartment(id: usize) -> FileVault {
    let dir = base_dir().join("compartments").join(id.to_string());
    FileVault::new(VaultConfig {
        base_dir: dir,
        tier1_file: "api_keys.json".into(),
        tier2_file: "vault.enc".into(),
    })
}

fn compartment_salt_path(id: usize) -> PathBuf {
    base_dir()
        .join("compartments")
        .join(id.to_string())
        .join("passphrase.salt")
}

fn compartment_wrapped_key_path(id: usize) -> PathBuf {
    base_dir()
        .join("compartments")
        .join(id.to_string())
        .join("passphrase_wrapped_key.enc")
}

/// Check if any compartment directory has a passphrase salt file.
fn has_any_passphrase_compartment() -> bool {
    for i in 0..SHARD_SLOTS {
        let salt_path = compartment_salt_path(i);
        if salt_path.exists() {
            let wrapped_path = compartment_wrapped_key_path(i);
            if wrapped_path.exists() {
                return true;
            }
        }
    }
    false
}

fn parse_label_arg(args: &[String], cmd: &str) -> String {
    parse_flag(args, "--label").unwrap_or_else(|| {
        eprintln!("Usage: sigillum fido2 {cmd} --label <LABEL>");
        process::exit(1);
    })
}

fn has_flag(args: &[String], flag: &str) -> bool {
    args.iter().any(|a| a == flag)
}

fn parse_flag(args: &[String], flag: &str) -> Option<String> {
    let mut i = 0;
    while i < args.len() {
        if args[i] == flag && i + 1 < args.len() {
            return Some(args[i + 1].clone());
        }
        i += 1;
    }
    None
}

fn require_arg(args: &[String], cmd: &str, placeholder: &str) -> String {
    if args.is_empty() {
        eprintln!("Usage: sigillum {cmd} {placeholder}");
        process::exit(1);
    }
    args[0].clone()
}

fn prompt_secret(prompt: &str) -> String {
    rpassword::prompt_password(prompt).unwrap_or_else(|e| {
        eprintln!("Failed to read secret: {e}");
        process::exit(1);
    })
}

fn prompt_optional_pin() -> Option<String> {
    let pin =
        rpassword::prompt_password("FIDO2 PIN (press Enter if this key does not require one): ")
            .unwrap_or_else(|e| {
                eprintln!("Failed to read FIDO2 PIN: {e}");
                process::exit(1);
            });
    if pin.is_empty() { None } else { Some(pin) }
}

fn prompt_passphrase() -> Zeroizing<String> {
    let p = rpassword::prompt_password("Passphrase: ").unwrap_or_else(|e| {
        eprintln!("Failed to read passphrase: {e}");
        process::exit(1);
    });
    Zeroizing::new(p)
}

fn prompt_passphrase_confirm() -> Zeroizing<String> {
    let p1 = prompt_passphrase();
    let p2 = rpassword::prompt_password("Confirm passphrase: ").unwrap_or_else(|e| {
        eprintln!("Failed to read passphrase: {e}");
        process::exit(1);
    });
    if p1.as_str() != p2.as_str() {
        eprintln!("Passphrases do not match.");
        process::exit(1);
    }
    p1
}
