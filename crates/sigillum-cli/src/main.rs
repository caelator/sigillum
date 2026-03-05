use std::io::{self, Write};
use std::path::PathBuf;
use std::process;

use argon2::Argon2;
use rand::rngs::OsRng;
use rand::RngCore;
use secrecy::ExposeSecret;
use sigillum_core::{FileVault, SecretStore, VaultConfig, VaultLifecycle};
use sigillum_fido2::Fido2Manager;
use sigillum_fido2::config::CompartmentDef;
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
        "fido2" => cmd_fido2(&args[2..]),
        "compartment" => cmd_compartment(&args[2..]),
        "daemon" => cmd_daemon(&args[2..]),
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
    setup             First-time setup wizard (FIDO2 or passphrase)
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

    compartment <CMD> Compartment management:
      list                   List compartments
      add --label <L> --threshold <T>   Add compartment
      remove --id <N>        Remove compartment
      init --id <N>          Initialize compartment with passphrase

    fido2 <CMD>       FIDO2 hardware key management:
      register --label <L>   Register a new hardware key
      list                   List registered keys
      remove --label <L>     Remove a hardware key
      status                 Show FIDO2 status
      unlock --taps <N>      Unlock a compartment via FIDO2

    daemon [--port N] Start HTTP daemon (default: localhost:9743)

    version           Show version
    help              Show this message"
    );
}

// ── Setup ────────────────────────────────────────────────────────

fn cmd_setup() {
    let mgr = fido2_manager();
    let config = mgr.load_config_raw();

    if !config.compartments.is_empty() || any_vault_exists(&config) {
        eprintln!("Vault already configured. Use 'sigillum compartment' to manage compartments.");
        process::exit(1);
    }

    println!("=== SIGILLUM SETUP WIZARD ===");
    println!();

    let device_count = sigillum_fido2::hid::detect_devices();
    if device_count > 0 {
        println!("Detected {device_count} FIDO2 device(s).");
        println!();
        println!("Choose setup method:");
        println!("  1) Hardware key (FIDO2) — recommended");
        println!("  2) Passphrase only");
        eprint!("Choice [1]: ");
        io::stderr().flush().unwrap();
        let mut choice = String::new();
        io::stdin().read_line(&mut choice).unwrap();

        if choice.trim() == "2" {
            setup_passphrase();
        } else {
            setup_fido2();
        }
    } else {
        println!("No FIDO2 device detected. Setting up with passphrase.");
        println!();
        setup_passphrase();
    }
}

fn setup_passphrase() {
    let mgr = fido2_manager();

    eprint!("Compartment label [default]: ");
    io::stderr().flush().unwrap();
    let mut label = String::new();
    io::stdin().read_line(&mut label).unwrap();
    let label = label.trim();
    let label = if label.is_empty() { "default" } else { label };

    let passphrase = prompt_passphrase_confirm();

    // Add compartment
    let def = CompartmentDef {
        id: 0,
        label: label.to_string(),
        threshold: 1,
        passphrase_mode: Some("wrapped".into()),
    };
    mgr.add_compartment(def).unwrap_or_else(|e| {
        eprintln!("Failed to add compartment: {e}");
        process::exit(1);
    });

    // Generate random master key
    let mut master_key = [0u8; 32];
    OsRng.fill_bytes(&mut master_key);

    // Initialize vault
    let vault = vault_for_compartment(0);
    if let Err(e) = vault.initialize(&master_key) {
        eprintln!("Failed to initialize vault: {e}");
        process::exit(1);
    }

    // Wrap master key with passphrase
    let (wrap_key, salt) = derive_key_from_passphrase(&passphrase);
    save_salt(&salt, &compartment_salt_path(0));
    save_wrapped_master_key(&master_key, &wrap_key, &compartment_wrapped_key_path(0));

    zeroize::Zeroize::zeroize(&mut master_key);

    println!();
    println!("Vault initialized. Compartment \"{label}\" created.");
    println!("Remember your passphrase — it cannot be recovered.");
}

fn setup_fido2() {
    let mgr = fido2_manager();

    println!();
    println!("Define compartments (each needs a unique tap-count threshold):");
    println!("  Default presets: hot=1, cold=2, legacy=3");
    eprint!("Use default presets? [Y/n]: ");
    io::stderr().flush().unwrap();
    let mut answer = String::new();
    io::stdin().read_line(&mut answer).unwrap();

    let compartments: Vec<(String, usize)> = if answer.trim().eq_ignore_ascii_case("n") {
        let mut comps = Vec::new();
        loop {
            eprint!("Compartment label (empty to finish): ");
            io::stderr().flush().unwrap();
            let mut label = String::new();
            io::stdin().read_line(&mut label).unwrap();
            let label = label.trim().to_string();
            if label.is_empty() { break; }

            eprint!("Threshold (tap count): ");
            io::stderr().flush().unwrap();
            let mut t = String::new();
            io::stdin().read_line(&mut t).unwrap();
            let threshold: usize = t.trim().parse().unwrap_or_else(|_| {
                eprintln!("Invalid threshold");
                process::exit(1);
            });
            comps.push((label, threshold));
        }
        if comps.is_empty() {
            eprintln!("At least one compartment required.");
            process::exit(1);
        }
        comps
    } else {
        vec![
            ("hot".into(), 1),
            ("cold".into(), 2),
            ("legacy".into(), 3),
        ]
    };

    // Register compartments
    for (i, (label, threshold)) in compartments.iter().enumerate() {
        let def = CompartmentDef {
            id: i,
            label: label.clone(),
            threshold: *threshold,
            passphrase_mode: None,
        };
        mgr.add_compartment(def).unwrap_or_else(|e| {
            eprintln!("Failed to add compartment: {e}");
            process::exit(1);
        });
    }

    let pin = prompt_pin();
    eprint!("Key label (e.g. 'yubikey-primary'): ");
    io::stderr().flush().unwrap();
    let mut label = String::new();
    io::stdin().read_line(&mut label).unwrap();
    let label = label.trim();

    if label.is_empty() {
        eprintln!("Label cannot be empty.");
        process::exit(1);
    }

    println!();
    println!("Touch your FIDO2 key now...");

    match mgr.register_key(&pin, label, &[]) {
        Ok(result) => {
            // Initialize each compartment vault
            for (comp_id, mk) in &result.compartment_keys {
                let vault = vault_for_compartment(*comp_id);
                if let Err(e) = vault.initialize(mk) {
                    eprintln!("Failed to initialize compartment {comp_id}: {e}");
                    process::exit(1);
                }
            }

            println!();
            println!("FIDO2 key '{label}' registered.");
            println!("{} compartment(s) created and initialized.", compartments.len());

            // Ask about passphrase backup
            eprint!("Set a backup passphrase for all compartments? [y/N]: ");
            io::stderr().flush().unwrap();
            let mut answer = String::new();
            io::stdin().read_line(&mut answer).unwrap();
            if answer.trim().eq_ignore_ascii_case("y") {
                let passphrase = prompt_passphrase_confirm();
                for (comp_id, mk) in &result.compartment_keys {
                    let (wrap_key, salt) = derive_key_from_passphrase(&passphrase);
                    save_salt(&salt, &compartment_salt_path(*comp_id));
                    save_wrapped_master_key(mk, &wrap_key, &compartment_wrapped_key_path(*comp_id));
                }
                let mut cfg = mgr.load_config_raw();
                for c in &mut cfg.compartments {
                    c.passphrase_mode = Some("wrapped".into());
                }
                let _ = mgr.save_config_raw(&cfg);
                println!("Passphrase backup configured for all compartments.");
            }

            // Ask about additional keys
            loop {
                eprint!("Register another hardware key? [y/N]: ");
                io::stderr().flush().unwrap();
                let mut answer = String::new();
                io::stdin().read_line(&mut answer).unwrap();
                if !answer.trim().eq_ignore_ascii_case("y") { break; }

                let pin = prompt_pin();
                eprint!("Key label: ");
                io::stderr().flush().unwrap();
                let mut next_label = String::new();
                io::stdin().read_line(&mut next_label).unwrap();
                let next_label = next_label.trim();

                // Need master keys for all compartments
                let mk_refs: Vec<(usize, &[u8; 32])> = result.compartment_keys.iter()
                    .map(|(id, mk)| (*id, &**mk))
                    .collect();

                println!("Touch your FIDO2 key now...");
                match mgr.register_key(&pin, next_label, &mk_refs) {
                    Ok(r) => println!("Key '{next_label}' registered ({} total).", r.total_keys),
                    Err(e) => eprintln!("Failed: {e}"),
                }
            }

            println!();
            println!("Setup complete.");
        }
        Err(e) => {
            eprintln!("FIDO2 registration failed: {e}");
            process::exit(1);
        }
    }
}

// ── Status ──────────────────────────────────────────────────────

fn cmd_status() {
    let mgr = fido2_manager();
    let fido_status = mgr.status();

    println!("=== SIGILLUM VAULT STATUS ===");
    println!("FIDO2 enabled:   {}", fido_status.enabled);
    println!("FIDO2 keys:      {}", fido_status.key_count);
    println!("Compartments:    {}", fido_status.compartments.len());

    for c in &fido_status.compartments {
        let vault = vault_for_compartment(c.id);
        let exists = vault.vault_exists();
        let unlocked = vault.is_unlocked();
        println!(
            "  [{id}] {label}: threshold={t}, exists={exists}, unlocked={unlocked}, passphrase={p}",
            id = c.id,
            label = c.label,
            t = c.threshold,
            p = c.has_passphrase,
        );
    }

    let device_count = sigillum_fido2::hid::detect_devices();
    println!("Devices present: {device_count}");
}

// ── Unlock ──────────────────────────────────────────────────────

fn cmd_unlock() {
    let mgr = fido2_manager();
    let config = mgr.load_config_raw();

    if config.compartments.is_empty() {
        eprintln!("No compartments configured. Run 'sigillum setup' first.");
        process::exit(1);
    }

    let has_fido = !config.keys.is_empty();
    let has_passphrase = config.compartments.iter().any(|c| c.passphrase_mode.is_some());

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
                unlock_passphrase(&config);
            } else {
                unlock_fido2(&mgr, &config);
            }
        } else {
            println!("No FIDO2 device detected. Using passphrase.");
            unlock_passphrase(&config);
        }
    } else if has_fido {
        unlock_fido2(&mgr, &config);
    } else if has_passphrase {
        unlock_passphrase(&config);
    } else {
        eprintln!("No unlock method configured.");
        process::exit(1);
    }
}

fn unlock_passphrase(config: &sigillum_fido2::config::Fido2Config) {
    let passphrase = prompt_passphrase();

    for comp in &config.compartments {
        if comp.passphrase_mode.as_deref() != Some("wrapped") { continue; }

        let salt = match std::fs::read(compartment_salt_path(comp.id)) {
            Ok(s) if s.len() == 32 => s,
            _ => continue,
        };

        let wrap_key = derive_key_with_salt(&passphrase, &salt);
        if let Some(master_key) = load_wrapped_master_key(&wrap_key, &compartment_wrapped_key_path(comp.id)) {
            let vault = vault_for_compartment(comp.id);
            vault.load_master_key(master_key);
            if vault.verify_master_key() {
                println!("Unlocked compartment \"{}\" (id={}).", comp.label, comp.id);
                return;
            }
            vault.zeroize_master_key();
        }
    }

    eprintln!("No compartment matched this passphrase.");
    process::exit(1);
}

fn unlock_fido2(mgr: &Fido2Manager, config: &sigillum_fido2::config::Fido2Config) {
    println!("Available compartments:");
    for c in &config.compartments {
        println!("  Tap {} key{} = \"{}\"", c.threshold, if c.threshold > 1 { "s" } else { "" }, c.label);
    }
    eprint!("Tap count: ");
    io::stderr().flush().unwrap();
    let mut taps_str = String::new();
    io::stdin().read_line(&mut taps_str).unwrap();
    let taps: usize = taps_str.trim().parse().unwrap_or_else(|_| {
        eprintln!("Invalid tap count");
        process::exit(1);
    });

    let pin = prompt_pin();
    println!("Touch your FIDO2 key now...");

    match mgr.authenticate_compartment(&[pin], taps, None) {
        Ok((comp_id, master_key)) => {
            let vault = vault_for_compartment(comp_id);
            vault.load_master_key(*master_key);
            if vault.verify_master_key() {
                let label = config.compartment_by_id(comp_id)
                    .map(|c| c.label.as_str())
                    .unwrap_or("unknown");
                println!("Unlocked compartment \"{}\" (id={}).", label, comp_id);
            } else {
                vault.zeroize_master_key();
                eprintln!("FIDO2 key does not match compartment vault.");
                process::exit(1);
            }
        }
        Err(e) => {
            eprintln!("FIDO2 unlock failed: {e}");
            process::exit(1);
        }
    }
}

fn cmd_lock() {
    let mgr = fido2_manager();
    let config = mgr.load_config_raw();
    for c in &config.compartments {
        let vault = vault_for_compartment(c.id);
        vault.zeroize_master_key();
    }
    println!("All compartments locked. Master keys zeroized.");
}

// ── Secrets (operate on first unlocked compartment) ──────────────

fn find_unlocked_vault() -> (usize, FileVault) {
    let mgr = fido2_manager();
    let config = mgr.load_config_raw();
    for c in &config.compartments {
        let vault = vault_for_compartment(c.id);
        if vault.is_unlocked() {
            return (c.id, vault);
        }
    }
    eprintln!("No compartment is unlocked. Run 'sigillum unlock' first.");
    process::exit(1);
}

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

fn cmd_get(args: &[String]) {
    let key = require_arg(args, "get", "<KEY>");
    let (_, vault) = find_unlocked_vault();
    match vault.get_secret(&key) {
        Some(val) => println!("{}", val.expose_secret()),
        None => {
            eprintln!("Secret '{key}' not found.");
            process::exit(1);
        }
    }
}

fn cmd_delete(args: &[String]) {
    let key = require_arg(args, "delete", "<KEY>");
    let (_, vault) = find_unlocked_vault();
    if let Err(e) = vault.delete_secret(&key) {
        eprintln!("Failed: {e}");
        process::exit(1);
    }
    println!("Secret '{key}' deleted.");
}

fn cmd_list() {
    let mgr = fido2_manager();
    let config = mgr.load_config_raw();
    let mut found_any = false;
    for c in &config.compartments {
        let vault = vault_for_compartment(c.id);
        let api_keys = vault.list_api_keys();
        if !api_keys.is_empty() {
            println!("=== [{}: {}] Tier 1 (API Keys) ===", c.id, c.label);
            for k in &api_keys { println!("  {k}"); }
            found_any = true;
        }
        if vault.is_unlocked() {
            let secrets = vault.list_secrets();
            if !secrets.is_empty() {
                println!("=== [{}: {}] Tier 2 (Encrypted Secrets) ===", c.id, c.label);
                for k in &secrets { println!("  {k}"); }
                found_any = true;
            }
        }
    }
    if !found_any {
        println!("No keys found (unlock a compartment to see Tier 2 secrets).");
    }
}

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

fn cmd_get_api(args: &[String]) {
    let key = require_arg(args, "get-api", "<KEY>");
    let (_, vault) = find_unlocked_vault();
    match vault.get_api_key(&key) {
        Some(val) => println!("{}", val.expose_secret()),
        None => {
            eprintln!("API key '{key}' not found.");
            process::exit(1);
        }
    }
}

fn cmd_delete_api(args: &[String]) {
    let key = require_arg(args, "delete-api", "<KEY>");
    let (_, vault) = find_unlocked_vault();
    if let Err(e) = vault.delete_api_key(&key) {
        eprintln!("Failed: {e}");
        process::exit(1);
    }
    println!("API key '{key}' deleted.");
}

// ── Compartment subcommands ─────────────────────────────────────

fn cmd_compartment(args: &[String]) {
    if args.is_empty() {
        eprintln!("Usage: sigillum compartment <list|add|remove|init>");
        process::exit(1);
    }

    match args[0].as_str() {
        "list" => compartment_list(),
        "add" => compartment_add(&args[1..]),
        "remove" => compartment_remove(&args[1..]),
        "init" => compartment_init(&args[1..]),
        other => {
            eprintln!("Unknown compartment command: {other}");
            process::exit(1);
        }
    }
}

fn compartment_list() {
    let mgr = fido2_manager();
    let config = mgr.load_config_raw();
    if config.compartments.is_empty() {
        println!("No compartments defined. Run 'sigillum setup' first.");
        return;
    }
    println!("=== Compartments ===");
    for c in &config.compartments {
        let vault = vault_for_compartment(c.id);
        let exists = vault.vault_exists();
        let unlocked = vault.is_unlocked();
        println!(
            "  [{id}] {label}: threshold={t}, initialized={exists}, unlocked={unlocked}",
            id = c.id, label = c.label, t = c.threshold,
        );
    }
}

fn compartment_add(args: &[String]) {
    let label = parse_flag(args, "--label").unwrap_or_else(|| {
        eprintln!("Usage: sigillum compartment add --label <L> --threshold <T>");
        process::exit(1);
    });
    let threshold: usize = parse_flag(args, "--threshold")
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| {
            eprintln!("Usage: sigillum compartment add --label <L> --threshold <T>");
            process::exit(1);
        });

    let mgr = fido2_manager();
    let config = mgr.load_config_raw();
    let id = config.next_compartment_id();

    let def = CompartmentDef {
        id,
        label: label.clone(),
        threshold,
        passphrase_mode: None,
    };

    match mgr.add_compartment(def) {
        Ok(()) => println!("Compartment \"{label}\" added (id={id}, threshold={threshold})."),
        Err(e) => {
            eprintln!("Failed: {e}");
            process::exit(1);
        }
    }
}

fn compartment_remove(args: &[String]) {
    let id: usize = parse_flag(args, "--id")
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| {
            eprintln!("Usage: sigillum compartment remove --id <N>");
            process::exit(1);
        });

    let mgr = fido2_manager();
    match mgr.remove_compartment(id) {
        Ok(()) => println!("Compartment {id} removed."),
        Err(e) => {
            eprintln!("Failed: {e}");
            process::exit(1);
        }
    }
}

fn compartment_init(args: &[String]) {
    let id: usize = parse_flag(args, "--id")
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| {
            eprintln!("Usage: sigillum compartment init --id <N>");
            process::exit(1);
        });

    let mgr = fido2_manager();
    let config = mgr.load_config_raw();
    let comp = config.compartment_by_id(id).unwrap_or_else(|| {
        eprintln!("Compartment {id} not found.");
        process::exit(1);
    });

    let vault = vault_for_compartment(id);
    if vault.vault_exists() {
        eprintln!("Compartment {id} already initialized.");
        process::exit(1);
    }

    let passphrase = prompt_passphrase_confirm();

    let mut master_key = [0u8; 32];
    OsRng.fill_bytes(&mut master_key);

    if let Err(e) = vault.initialize(&master_key) {
        eprintln!("Failed: {e}");
        process::exit(1);
    }

    let (wrap_key, salt) = derive_key_from_passphrase(&passphrase);
    save_salt(&salt, &compartment_salt_path(id));
    save_wrapped_master_key(&master_key, &wrap_key, &compartment_wrapped_key_path(id));

    // Update passphrase_mode
    if comp.passphrase_mode.is_none() {
        let mut cfg = mgr.load_config_raw();
        if let Some(c) = cfg.compartments.iter_mut().find(|c| c.id == id) {
            c.passphrase_mode = Some("wrapped".into());
        }
        let _ = mgr.save_config_raw(&cfg);
    }

    zeroize::Zeroize::zeroize(&mut master_key);
    println!("Compartment {} initialized.", comp.label);
}

// ── FIDO2 subcommands ───────────────────────────────────────────

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
        "unlock" => {
            let config = mgr.load_config_raw();
            unlock_fido2(&mgr, &config);
        }
        other => {
            eprintln!("Unknown fido2 command: {other}");
            process::exit(1);
        }
    }
}

fn fido2_register(mgr: &Fido2Manager, args: &[String]) {
    let label = parse_label_arg(args, "register");
    let config = mgr.load_config_raw();

    if config.compartments.is_empty() {
        eprintln!("No compartments defined. Run 'sigillum setup' first.");
        process::exit(1);
    }

    // Need master keys for all compartments
    let mut master_keys: Vec<(usize, Zeroizing<[u8; 32]>)> = Vec::new();
    for c in &config.compartments {
        let vault = vault_for_compartment(c.id);
        if !vault.is_unlocked() {
            eprintln!("All compartments must be unlocked to register a new key.");
            eprintln!("Compartment {} ({}) is locked.", c.id, c.label);
            process::exit(1);
        }
        match vault.extract_master_key() {
            Some(mk) => master_keys.push((c.id, mk)),
            None => {
                eprintln!("Cannot extract master key from compartment {}.", c.id);
                process::exit(1);
            }
        }
    }

    let pin = prompt_pin();
    println!("Touch your FIDO2 key now...");

    let mk_refs: Vec<(usize, &[u8; 32])> = master_keys.iter()
        .map(|(id, mk)| (*id, &**mk))
        .collect();

    match mgr.register_key(&pin, &label, &mk_refs) {
        Ok(result) => println!("Key '{label}' registered ({} total).", result.total_keys),
        Err(e) => {
            eprintln!("Registration failed: {e}");
            process::exit(1);
        }
    }
}

fn fido2_list(mgr: &Fido2Manager) {
    let keys = mgr.list_keys();
    if keys.is_empty() {
        println!("No FIDO2 keys registered.");
        return;
    }
    println!("=== Registered FIDO2 Keys ===");
    for k in &keys {
        println!(
            "  {} ({}...) — {} — compartments: {:?}",
            k.label, k.credential_id_short, k.registered_at, k.compartment_ids,
        );
    }
}

fn fido2_remove(mgr: &Fido2Manager, args: &[String]) {
    let label = parse_label_arg(args, "remove");

    // Need master keys for all compartments
    let config = mgr.load_config_raw();
    let mut master_keys: Vec<(usize, Zeroizing<[u8; 32]>)> = Vec::new();
    for c in &config.compartments {
        let vault = vault_for_compartment(c.id);
        if !vault.is_unlocked() {
            eprintln!("All compartments must be unlocked to remove a key.");
            process::exit(1);
        }
        match vault.extract_master_key() {
            Some(mk) => master_keys.push((c.id, mk)),
            None => {
                eprintln!("Cannot extract master key from compartment {}.", c.id);
                process::exit(1);
            }
        }
    }

    let pin = prompt_pin();
    println!("Tap remaining keys to re-encrypt shards...");

    let mk_refs: Vec<(usize, &[u8; 32])> = master_keys.iter()
        .map(|(id, mk)| (*id, &**mk))
        .collect();

    match mgr.remove_key(&label, &mk_refs, &pin) {
        Ok(()) => println!("Key '{label}' removed."),
        Err(e) => {
            eprintln!("Removal failed: {e}");
            process::exit(1);
        }
    }
}

fn fido2_status(mgr: &Fido2Manager) {
    let s = mgr.status();
    let device_count = sigillum_fido2::hid::detect_devices();

    println!("=== FIDO2 STATUS ===");
    println!("Enabled:         {}", s.enabled);
    println!("Registered keys: {}", s.key_count);
    println!("Compartments:    {}", s.compartments.len());
    for c in &s.compartments {
        println!("  [{id}] {label}: threshold={t}", id = c.id, label = c.label, t = c.threshold);
    }
    println!("Devices present: {device_count}");
}

// ── Daemon ──────────────────────────────────────────────────────

fn cmd_daemon(args: &[String]) {
    let mut port: u16 = 9743;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--port" | "-p" => {
                i += 1;
                port = args
                    .get(i)
                    .and_then(|s| s.parse().ok())
                    .unwrap_or_else(|| {
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
    base_dir().join("compartments").join(id.to_string()).join("passphrase.salt")
}

fn compartment_wrapped_key_path(id: usize) -> PathBuf {
    base_dir().join("compartments").join(id.to_string()).join("passphrase_wrapped_key.enc")
}

fn any_vault_exists(config: &sigillum_fido2::config::Fido2Config) -> bool {
    config.compartments.iter().any(|c| vault_for_compartment(c.id).vault_exists())
}

fn parse_label_arg(args: &[String], cmd: &str) -> String {
    parse_flag(args, "--label").unwrap_or_else(|| {
        eprintln!("Usage: sigillum fido2 {cmd} --label <LABEL>");
        process::exit(1);
    })
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
    eprint!("{prompt}");
    io::stderr().flush().unwrap();
    let mut line = String::new();
    io::stdin().read_line(&mut line).unwrap();
    line.trim_end().to_string()
}

fn prompt_pin() -> String {
    rpassword::prompt_password("FIDO2 PIN: ").unwrap_or_else(|e| {
        eprintln!("Failed to read PIN: {e}");
        process::exit(1);
    })
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

fn save_salt(salt: &[u8; 32], path: &std::path::Path) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Err(e) = std::fs::write(path, salt) {
        eprintln!("Failed to save salt: {e}");
        process::exit(1);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
}

fn save_wrapped_master_key(master_key: &[u8; 32], wrap_key: &[u8; 32], path: &std::path::Path) {
    use aes_gcm::aead::Aead;
    use aes_gcm::{Aes256Gcm, KeyInit, Nonce};

    let cipher = Aes256Gcm::new_from_slice(wrap_key).expect("wrap key length");
    let mut nonce_bytes = [0u8; 12];
    OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ciphertext = cipher
        .encrypt(nonce, master_key.as_ref())
        .expect("wrap encryption");

    let mut output = Vec::with_capacity(12 + ciphertext.len());
    output.extend_from_slice(&nonce_bytes);
    output.extend_from_slice(&ciphertext);

    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Err(e) = std::fs::write(path, &output) {
        eprintln!("Failed to save wrapped key: {e}");
        process::exit(1);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
}

fn load_wrapped_master_key(wrap_key: &[u8; 32], path: &std::path::Path) -> Option<[u8; 32]> {
    use aes_gcm::aead::Aead;
    use aes_gcm::{Aes256Gcm, KeyInit, Nonce};

    let data = std::fs::read(path).ok()?;
    if data.len() < 12 { return None; }
    let (nonce_bytes, ciphertext) = data.split_at(12);
    let cipher = Aes256Gcm::new_from_slice(wrap_key).ok()?;
    let plaintext = cipher
        .decrypt(Nonce::from_slice(nonce_bytes), ciphertext)
        .ok()?;
    if plaintext.len() < 32 { return None; }
    let mut key = [0u8; 32];
    key.copy_from_slice(&plaintext[..32]);
    Some(key)
}

fn derive_key_from_passphrase(passphrase: &str) -> ([u8; 32], [u8; 32]) {
    let mut salt = [0u8; 32];
    OsRng.fill_bytes(&mut salt);
    let key = derive_key_with_salt(passphrase, &salt);
    (key, salt)
}

fn derive_key_with_salt(passphrase: &str, salt: &[u8]) -> [u8; 32] {
    let mut key = [0u8; 32];
    let argon2 = Argon2::new(
        argon2::Algorithm::Argon2id,
        argon2::Version::V0x13,
        argon2::Params::new(65536, 3, 1, Some(32)).unwrap(),
    );
    argon2
        .hash_password_into(passphrase.as_bytes(), salt, &mut key)
        .expect("Argon2id derivation failed");
    key
}
