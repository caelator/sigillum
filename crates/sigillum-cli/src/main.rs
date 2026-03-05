use std::io::{self, Write};
use std::process;

use argon2::Argon2;
use rand::rngs::OsRng;
use rand::RngCore;
use secrecy::ExposeSecret;
use sigillum_core::{FileVault, SecretStore, VaultConfig, VaultLifecycle};
use sigillum_fido2::Fido2Manager;
use zeroize::Zeroizing;

fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 2 {
        print_usage();
        process::exit(1);
    }

    let vault = FileVault::new(VaultConfig::default());

    match args[1].as_str() {
        "init" => cmd_init(&vault),
        "setup" => cmd_setup(&vault),
        "status" => cmd_status(&vault),
        "unlock" => cmd_unlock(&vault),
        "lock" => cmd_lock(&vault),
        "set" => cmd_set(&vault, &args[2..]),
        "get" => cmd_get(&vault, &args[2..]),
        "delete" => cmd_delete(&vault, &args[2..]),
        "list" => cmd_list(&vault),
        "set-api" => cmd_set_api(&vault, &args[2..]),
        "get-api" => cmd_get_api(&vault, &args[2..]),
        "delete-api" => cmd_delete_api(&vault, &args[2..]),
        "fido2" => cmd_fido2(&vault, &args[2..]),
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
    init              Initialize a new vault with passphrase only
    status            Show vault status
    unlock            Unlock vault (auto-detects FIDO2 or passphrase)
    lock              Lock vault (zeroize master key from memory)

    set <KEY>         Store a Tier 2 secret (encrypted, requires unlock)
    get <KEY>         Retrieve a Tier 2 secret
    delete <KEY>      Delete a Tier 2 secret
    list              List all keys (both tiers)

    set-api <KEY>     Store a Tier 1 API key (plaintext, no unlock needed)
    get-api <KEY>     Retrieve a Tier 1 API key
    delete-api <KEY>  Delete a Tier 1 API key

    fido2 <CMD>       FIDO2 hardware key management:
      register --label <L>   Register a new hardware key
      list                   List registered keys
      remove --label <L>     Remove a hardware key
      set-quorum <N>         Set quorum threshold
      status                 Show FIDO2 status
      unlock                 Unlock via FIDO2 quorum

    daemon [--port N] Start HTTP daemon (default: localhost:9743)

    version           Show version
    help              Show this message"
    );
}

// ── Commands ──────────────────────────────────────────────────────

fn cmd_setup(vault: &FileVault) {
    if vault.vault_exists() {
        eprintln!("Vault already exists at ~/.sigillum/");
        eprintln!("Use 'sigillum fido2 register' to add keys to an existing vault.");
        process::exit(1);
    }

    println!("=== SIGILLUM SETUP WIZARD ===");
    println!();

    // Check for FIDO2 device
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
        let choice = choice.trim();

        if choice == "2" {
            setup_passphrase(vault);
        } else {
            setup_fido2(vault);
        }
    } else {
        println!("No FIDO2 device detected. Setting up with passphrase.");
        println!("(Insert a FIDO2 key and re-run 'sigillum setup' for hardware key unlock.)");
        println!();
        setup_passphrase(vault);
    }
}

fn setup_passphrase(vault: &FileVault) {
    let passphrase = prompt_passphrase_confirm();
    let (master_key, salt) = derive_key_from_passphrase(&passphrase);

    if let Err(e) = vault.initialize(&master_key) {
        eprintln!("Failed to initialize vault: {e}");
        process::exit(1);
    }

    save_salt(&salt);
    println!();
    println!("Vault initialized with passphrase at ~/.sigillum/");
    println!("Remember your passphrase — it cannot be recovered.");
}

fn setup_fido2(vault: &FileVault) {
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

    let mgr = fido2_manager();
    match mgr.register_key(&pin, label, None) {
        Ok(result) => {
            // Initialize vault with the generated master key
            if let Err(e) = vault.initialize(&result.master_key) {
                eprintln!("Failed to initialize vault: {e}");
                process::exit(1);
            }

            println!();
            println!("FIDO2 key '{label}' registered.");
            println!("Vault initialized at ~/.sigillum/");
            println!();

            // Ask about passphrase fallback
            eprint!("Set a passphrase as backup? [y/N]: ");
            io::stderr().flush().unwrap();
            let mut answer = String::new();
            io::stdin().read_line(&mut answer).unwrap();
            if answer.trim().eq_ignore_ascii_case("y") {
                setup_passphrase_fallback(vault, &result.master_key);
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

                let pin = prompt_pin();
                eprint!("Key label: ");
                io::stderr().flush().unwrap();
                let mut next_label = String::new();
                io::stdin().read_line(&mut next_label).unwrap();
                let next_label = next_label.trim();

                println!("Touch your FIDO2 key now...");
                match mgr.register_key(&pin, next_label, Some(&result.master_key)) {
                    Ok(r) => println!("Key '{next_label}' registered ({} total).", r.total_keys),
                    Err(e) => eprintln!("Failed: {e}"),
                }
            }

            // Quorum threshold
            let status = mgr.status();
            if status.key_count > 1 {
                println!();
                println!("{} keys registered. Current quorum threshold: 1", status.key_count);
                eprint!("Set quorum threshold (1-{}): ", status.key_count);
                io::stderr().flush().unwrap();
                let mut threshold_str = String::new();
                io::stdin().read_line(&mut threshold_str).unwrap();
                if let Ok(t) = threshold_str.trim().parse::<usize>() {
                    if let Err(e) = mgr.set_quorum(t) {
                        eprintln!("Failed to set quorum: {e}");
                    } else {
                        println!("Quorum set to {t}-of-{}.", status.key_count);
                    }
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

fn setup_passphrase_fallback(_vault: &FileVault, master_key: &[u8; 32]) {
    let passphrase = prompt_passphrase_confirm();
    let (wrap_key, salt) = derive_key_from_passphrase(&passphrase);

    // Wrap the master key with the passphrase-derived key
    save_wrapped_master_key(master_key, &wrap_key);
    save_salt(&salt);

    // Update FIDO2 config to record "both" mode
    let mgr = fido2_manager();
    let mut config = mgr.load_config_raw();
    config.unlock_method = "both".into();
    config.passphrase_mode = Some("wrapped".into());
    let _ = mgr.save_config_raw(&config);

    println!("Passphrase fallback configured.");
}

fn cmd_init(vault: &FileVault) {
    if vault.vault_exists() {
        eprintln!("Vault already exists at ~/.sigillum/vault.enc");
        eprintln!("To reinitialize, delete the existing vault first.");
        process::exit(1);
    }

    let passphrase = prompt_passphrase_confirm();
    let (master_key, salt) = derive_key_from_passphrase(&passphrase);

    if let Err(e) = vault.initialize(&master_key) {
        eprintln!("Failed to initialize vault: {e}");
        process::exit(1);
    }

    save_salt(&salt);
    println!("Vault initialized at ~/.sigillum/");
    println!("Remember your passphrase — it cannot be recovered.");
}

fn cmd_status(vault: &FileVault) {
    let exists = vault.vault_exists();
    let unlocked = vault.is_unlocked();
    let api_keys = vault.list_api_keys();

    println!("=== SIGILLUM VAULT STATUS ===");
    println!("Vault exists:    {exists}");
    println!("Vault unlocked:  {unlocked}");
    println!("Tier 1 keys:     {}", api_keys.len());

    if unlocked {
        let secrets = vault.list_secrets();
        println!("Tier 2 secrets:  {}", secrets.len());
    } else {
        println!("Tier 2 secrets:  (locked)");
    }

    // FIDO2 status
    let mgr = fido2_manager();
    let fido_status = mgr.status();
    println!();
    println!("FIDO2 enabled:   {}", fido_status.enabled);
    println!("FIDO2 keys:      {}", fido_status.key_count);
    println!("Quorum:          {}", fido_status.quorum_threshold);
    println!("Unlock method:   {}", fido_status.unlock_method);
}

fn cmd_unlock(vault: &FileVault) {
    if vault.is_unlocked() {
        println!("Vault is already unlocked.");
        return;
    }

    if !vault.vault_exists() {
        eprintln!("No vault found. Run 'sigillum setup' first.");
        process::exit(1);
    }

    // Auto-detect unlock method
    let mgr = fido2_manager();
    let method = mgr.unlock_method();

    match method.as_str() {
        "fido2" => unlock_fido2(vault, &mgr),
        "both" => {
            // Check if FIDO2 device is present
            if sigillum_fido2::hid::is_device_present() {
                println!("FIDO2 device detected. Use hardware key or passphrase?");
                println!("  1) Hardware key");
                println!("  2) Passphrase");
                eprint!("Choice [1]: ");
                io::stderr().flush().unwrap();
                let mut choice = String::new();
                io::stdin().read_line(&mut choice).unwrap();
                if choice.trim() == "2" {
                    unlock_passphrase_wrapped(vault);
                } else {
                    unlock_fido2(vault, &mgr);
                }
            } else {
                println!("No FIDO2 device detected. Using passphrase.");
                unlock_passphrase_wrapped(vault);
            }
        }
        _ => unlock_passphrase_direct(vault),
    }
}

fn unlock_passphrase_direct(vault: &FileVault) {
    let passphrase = prompt_passphrase();
    let salt = match std::fs::read(salt_path()) {
        Ok(s) if s.len() == 32 => s,
        _ => {
            eprintln!("Cannot read salt file. Vault may be corrupted.");
            process::exit(1);
        }
    };

    let master_key = derive_key_with_salt(&passphrase, &salt);
    vault.load_master_key(master_key);

    if vault.verify_master_key() {
        println!("Vault unlocked.");
    } else {
        vault.zeroize_master_key();
        eprintln!("Wrong passphrase.");
        process::exit(1);
    }
}

fn unlock_passphrase_wrapped(vault: &FileVault) {
    let passphrase = prompt_passphrase();
    let salt = match std::fs::read(salt_path()) {
        Ok(s) if s.len() == 32 => s,
        _ => {
            eprintln!("Cannot read salt file. Vault may be corrupted.");
            process::exit(1);
        }
    };

    let wrap_key = derive_key_with_salt(&passphrase, &salt);
    match load_wrapped_master_key(&wrap_key) {
        Some(master_key) => {
            vault.load_master_key(master_key);
            if vault.verify_master_key() {
                println!("Vault unlocked (passphrase).");
            } else {
                vault.zeroize_master_key();
                eprintln!("Decrypted key does not match vault. Vault may be corrupted.");
                process::exit(1);
            }
        }
        None => {
            eprintln!("Wrong passphrase or corrupted wrapped key file.");
            process::exit(1);
        }
    }
}

fn unlock_fido2(vault: &FileVault, mgr: &Fido2Manager) {
    let pin = prompt_pin();
    println!("Touch your FIDO2 key now...");

    match mgr.authenticate_quorum(&[pin], None) {
        Ok(master_key) => {
            vault.load_master_key(*master_key);
            if vault.verify_master_key() {
                println!("Vault unlocked (FIDO2).");
            } else {
                vault.zeroize_master_key();
                eprintln!("FIDO2 key does not match vault. Keys may have changed.");
                process::exit(1);
            }
        }
        Err(e) => {
            eprintln!("FIDO2 unlock failed: {e}");
            process::exit(1);
        }
    }
}

fn cmd_lock(vault: &FileVault) {
    vault.zeroize_master_key();
    println!("Vault locked. Master key zeroized.");
}

fn cmd_set(vault: &FileVault, args: &[String]) {
    let key = require_arg(args, "set", "<KEY>");
    if !vault.is_unlocked() {
        eprintln!("Vault is locked. Run 'sigillum unlock' first.");
        process::exit(1);
    }

    let value = prompt_secret("Value: ");
    if let Err(e) = vault.set_secret(&key, &value) {
        eprintln!("Failed: {e}");
        process::exit(1);
    }
    println!("Secret '{key}' stored (Tier 2, encrypted).");
}

fn cmd_get(vault: &FileVault, args: &[String]) {
    let key = require_arg(args, "get", "<KEY>");
    if !vault.is_unlocked() {
        eprintln!("Vault is locked. Run 'sigillum unlock' first.");
        process::exit(1);
    }

    match vault.get_secret(&key) {
        Some(val) => println!("{}", val.expose_secret()),
        None => {
            eprintln!("Secret '{key}' not found.");
            process::exit(1);
        }
    }
}

fn cmd_delete(vault: &FileVault, args: &[String]) {
    let key = require_arg(args, "delete", "<KEY>");
    if !vault.is_unlocked() {
        eprintln!("Vault is locked. Run 'sigillum unlock' first.");
        process::exit(1);
    }

    if let Err(e) = vault.delete_secret(&key) {
        eprintln!("Failed: {e}");
        process::exit(1);
    }
    println!("Secret '{key}' deleted.");
}

fn cmd_list(vault: &FileVault) {
    let api_keys = vault.list_api_keys();
    if !api_keys.is_empty() {
        println!("=== Tier 1 (API Keys) ===");
        for k in &api_keys {
            println!("  {k}");
        }
    }

    if vault.is_unlocked() {
        let secrets = vault.list_secrets();
        if !secrets.is_empty() {
            println!("=== Tier 2 (Encrypted Secrets) ===");
            for k in &secrets {
                println!("  {k}");
            }
        }
        if api_keys.is_empty() && secrets.is_empty() {
            println!("Vault is empty.");
        }
    } else {
        println!("(Tier 2 secrets hidden — vault is locked)");
    }
}

fn cmd_set_api(vault: &FileVault, args: &[String]) {
    let key = require_arg(args, "set-api", "<KEY>");
    let value = prompt_secret("Value: ");
    if let Err(e) = vault.set_api_key(&key, &value) {
        eprintln!("Failed: {e}");
        process::exit(1);
    }
    println!("API key '{key}' stored (Tier 1, plaintext).");
}

fn cmd_get_api(vault: &FileVault, args: &[String]) {
    let key = require_arg(args, "get-api", "<KEY>");
    match vault.get_api_key(&key) {
        Some(val) => println!("{}", val.expose_secret()),
        None => {
            eprintln!("API key '{key}' not found.");
            process::exit(1);
        }
    }
}

fn cmd_delete_api(vault: &FileVault, args: &[String]) {
    let key = require_arg(args, "delete-api", "<KEY>");
    if let Err(e) = vault.delete_api_key(&key) {
        eprintln!("Failed: {e}");
        process::exit(1);
    }
    println!("API key '{key}' deleted.");
}

// ── FIDO2 subcommands ────────────────────────────────────────────

fn cmd_fido2(vault: &FileVault, args: &[String]) {
    if args.is_empty() {
        eprintln!("Usage: sigillum fido2 <register|list|remove|set-quorum|status|unlock>");
        process::exit(1);
    }

    let mgr = fido2_manager();

    match args[0].as_str() {
        "register" => fido2_register(vault, &mgr, &args[1..]),
        "list" => fido2_list(&mgr),
        "remove" => fido2_remove(vault, &mgr, &args[1..]),
        "set-quorum" => fido2_set_quorum(&mgr, &args[1..]),
        "status" => fido2_status(&mgr),
        "unlock" => unlock_fido2(vault, &mgr),
        other => {
            eprintln!("Unknown fido2 command: {other}");
            process::exit(1);
        }
    }
}

fn fido2_register(vault: &FileVault, mgr: &Fido2Manager, args: &[String]) {
    let label = parse_label_arg(args, "register");

    if !vault.vault_exists() {
        eprintln!("No vault found. Run 'sigillum setup' first.");
        process::exit(1);
    }

    // Need master key for Nth key registration
    let existing_mk = if mgr.is_enabled() {
        if !vault.is_unlocked() {
            eprintln!("Vault must be unlocked to add another FIDO2 key.");
            process::exit(1);
        }
        vault.extract_master_key()
    } else {
        // First FIDO2 key on an existing passphrase vault
        if !vault.is_unlocked() {
            eprintln!("Vault must be unlocked to register first FIDO2 key.");
            process::exit(1);
        }
        vault.extract_master_key()
    };

    let pin = prompt_pin();
    println!("Touch your FIDO2 key now...");

    let mk_ref = existing_mk.as_ref().map(|k| &**k);
    match mgr.register_key(&pin, &label, mk_ref) {
        Ok(result) => {
            println!("Key '{label}' registered ({} total).", result.total_keys);
        }
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
        println!("  {} ({}...) — {}", k.label, k.credential_id_short, k.registered_at);
    }
    println!();
    println!("Quorum threshold: {}", mgr.quorum_threshold());
}

fn fido2_remove(vault: &FileVault, mgr: &Fido2Manager, args: &[String]) {
    let label = parse_label_arg(args, "remove");

    if !vault.is_unlocked() {
        eprintln!("Vault must be unlocked to remove a key.");
        process::exit(1);
    }

    let master_key = vault.extract_master_key().unwrap_or_else(|| {
        eprintln!("Cannot extract master key.");
        process::exit(1);
    });

    let pin = prompt_pin();
    println!("Tap remaining keys to re-encrypt shards...");

    match mgr.remove_key(&label, &master_key, &pin) {
        Ok(()) => println!("Key '{label}' removed."),
        Err(e) => {
            eprintln!("Removal failed: {e}");
            process::exit(1);
        }
    }
}

fn fido2_set_quorum(mgr: &Fido2Manager, args: &[String]) {
    if args.is_empty() {
        eprintln!("Usage: sigillum fido2 set-quorum <N>");
        process::exit(1);
    }

    let threshold: usize = args[0].parse().unwrap_or_else(|_| {
        eprintln!("Invalid threshold: {}", args[0]);
        process::exit(1);
    });

    match mgr.set_quorum(threshold) {
        Ok(()) => println!("Quorum threshold set to {threshold}."),
        Err(e) => {
            eprintln!("Failed: {e}");
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
    println!("Quorum:          {}", s.quorum_threshold);
    println!("Unlock method:   {}", s.unlock_method);
    println!("Devices present: {device_count}");
}

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
    let config = VaultConfig::default();

    let rt = tokio::runtime::Runtime::new().unwrap_or_else(|e| {
        eprintln!("Failed to start async runtime: {e}");
        process::exit(1);
    });

    if let Err(e) = rt.block_on(sigillum_daemon::run(addr, config)) {
        eprintln!("Daemon error: {e}");
        process::exit(1);
    }
}

// ── Helpers ───────────────────────────────────────────────────────

fn fido2_manager() -> Fido2Manager {
    let config_path = VaultConfig::default().base_dir.join("fido2_keys.json");
    Fido2Manager::new(config_path)
}

fn parse_label_arg(args: &[String], cmd: &str) -> String {
    let mut i = 0;
    while i < args.len() {
        if (args[i] == "--label" || args[i] == "-l") && i + 1 < args.len() {
            return args[i + 1].clone();
        }
        i += 1;
    }
    eprintln!("Usage: sigillum fido2 {cmd} --label <LABEL>");
    process::exit(1);
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

fn salt_path() -> std::path::PathBuf {
    VaultConfig::default().base_dir.join("passphrase.salt")
}

fn wrapped_key_path() -> std::path::PathBuf {
    VaultConfig::default().base_dir.join("passphrase_wrapped_key.enc")
}

fn save_salt(salt: &[u8; 32]) {
    let salt_path = salt_path();
    if let Some(dir) = salt_path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Err(e) = std::fs::write(&salt_path, salt) {
        eprintln!("Failed to save salt: {e}");
        process::exit(1);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&salt_path, std::fs::Permissions::from_mode(0o600));
    }
}

fn save_wrapped_master_key(master_key: &[u8; 32], wrap_key: &[u8; 32]) {
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

    let path = wrapped_key_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Err(e) = std::fs::write(&path, &output) {
        eprintln!("Failed to save wrapped key: {e}");
        process::exit(1);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
}

fn load_wrapped_master_key(wrap_key: &[u8; 32]) -> Option<[u8; 32]> {
    use aes_gcm::aead::Aead;
    use aes_gcm::{Aes256Gcm, KeyInit, Nonce};

    let data = std::fs::read(wrapped_key_path()).ok()?;
    if data.len() < 12 {
        return None;
    }
    let (nonce_bytes, ciphertext) = data.split_at(12);
    let cipher = Aes256Gcm::new_from_slice(wrap_key).ok()?;
    let plaintext = cipher
        .decrypt(Nonce::from_slice(nonce_bytes), ciphertext)
        .ok()?;
    if plaintext.len() < 32 {
        return None;
    }
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
        argon2::Params::new(65536, 3, 1, Some(32)).unwrap(), // 64MB, 3 iterations, 1 thread
    );
    argon2
        .hash_password_into(passphrase.as_bytes(), salt, &mut key)
        .expect("Argon2id derivation failed");
    key
}
