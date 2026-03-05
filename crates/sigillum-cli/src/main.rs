use std::io::{self, Write};
use std::process;

use argon2::Argon2;
use rand::rngs::OsRng;
use rand::RngCore;
use secrecy::ExposeSecret;
use sigillum_core::{FileVault, SecretStore, VaultConfig, VaultLifecycle};
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
    init              Initialize a new vault (generates master key)
    status            Show vault status
    unlock            Unlock vault with passphrase
    lock              Lock vault (zeroize master key from memory)

    set <KEY>         Store a Tier 2 secret (encrypted, requires unlock)
    get <KEY>         Retrieve a Tier 2 secret
    delete <KEY>      Delete a Tier 2 secret
    list              List all keys (both tiers)

    set-api <KEY>     Store a Tier 1 API key (plaintext, no unlock needed)
    get-api <KEY>     Retrieve a Tier 1 API key
    delete-api <KEY>  Delete a Tier 1 API key

    daemon [--port N] Start HTTP daemon (default: localhost:9743)

    version           Show version
    help              Show this message"
    );
}

// ── Commands ──────────────────────────────────────────────────────

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

    // Save the salt so we can re-derive on unlock
    let salt_path = salt_path();
    if let Some(dir) = salt_path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Err(e) = std::fs::write(&salt_path, &salt) {
        eprintln!("Failed to save salt: {e}");
        process::exit(1);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&salt_path, std::fs::Permissions::from_mode(0o600));
    }

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
}

fn cmd_unlock(vault: &FileVault) {
    if vault.is_unlocked() {
        println!("Vault is already unlocked.");
        return;
    }

    if !vault.vault_exists() {
        eprintln!("No vault found. Run 'sigillum init' first.");
        process::exit(1);
    }

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

fn prompt_passphrase() -> Zeroizing<String> {
    eprint!("Passphrase: ");
    io::stderr().flush().unwrap();
    let mut line = String::new();
    io::stdin().read_line(&mut line).unwrap();
    Zeroizing::new(line.trim_end().to_string())
}

fn prompt_passphrase_confirm() -> Zeroizing<String> {
    let p1 = prompt_passphrase();
    eprint!("Confirm passphrase: ");
    io::stderr().flush().unwrap();
    let mut p2 = String::new();
    io::stdin().read_line(&mut p2).unwrap();
    let p2 = p2.trim_end();
    if p1.as_str() != p2 {
        eprintln!("Passphrases do not match.");
        process::exit(1);
    }
    p1
}

fn salt_path() -> std::path::PathBuf {
    VaultConfig::default().base_dir.join("passphrase.salt")
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
