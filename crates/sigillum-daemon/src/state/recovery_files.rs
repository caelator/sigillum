use std::path::{Path, PathBuf};

pub(super) fn stash_snapshot_placeholder_ops(
    base_dir: &Path,
) -> Result<Option<PathBuf>, std::io::Error> {
    let rollback = snapshot_temp_path(base_dir, "rollback");
    if !rollback.exists() || !snapshot_placeholder_dir(base_dir)? {
        return Ok(None);
    }

    let ops_dir = base_dir.join(".ops");
    if !ops_dir.exists() {
        return Ok(None);
    }

    let preserved_ops = snapshot_temp_path(base_dir, "ops-preserved");
    if preserved_ops.exists() {
        std::fs::remove_dir_all(&preserved_ops)?;
    }
    std::fs::rename(&ops_dir, &preserved_ops)?;
    std::fs::remove_dir_all(base_dir)?;
    Ok(Some(preserved_ops))
}

pub(super) fn restore_stashed_ops_dir(
    base_dir: &Path,
    preserved_ops: &Path,
) -> Result<(), std::io::Error> {
    if !preserved_ops.exists() {
        return Ok(());
    }
    std::fs::create_dir_all(base_dir)?;
    let target = base_dir.join(".ops");
    if target.exists() {
        std::fs::remove_dir_all(&target)?;
    }
    std::fs::rename(preserved_ops, target)
}

fn snapshot_placeholder_dir(base_dir: &Path) -> Result<bool, std::io::Error> {
    let mut entries = match std::fs::read_dir(base_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };

    entries.try_fold(true, |is_placeholder, entry| {
        let entry = entry?;
        Ok(is_placeholder && entry.file_name() == ".ops")
    })
}

fn snapshot_temp_path(base_dir: &Path, suffix: &str) -> PathBuf {
    let parent = base_dir.parent().unwrap_or(Path::new("."));
    let name = base_dir
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "sigillum".into());
    parent.join(format!(".{name}.{suffix}"))
}

pub(crate) fn recover_compartment_replacements(base_dir: &Path) -> Result<(), std::io::Error> {
    let compartments_dir = base_dir.join("compartments");
    let entries = match std::fs::read_dir(&compartments_dir) {
        Ok(entries) => entries.collect::<Result<Vec<_>, _>>()?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };

    let mut compartment_ids = std::collections::BTreeSet::new();
    for entry in entries {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let candidate = name
            .split('.')
            .next()
            .and_then(|prefix| prefix.parse::<usize>().ok());
        if let Some(id) = candidate {
            compartment_ids.insert(id);
        }
    }

    for id in compartment_ids {
        let live = compartments_dir.join(id.to_string());
        let replacement = live.with_extension("replacing");
        let rollback = live.with_extension("rollback");
        if live.exists() {
            if rollback.exists() {
                std::fs::remove_dir_all(&rollback)?;
            }
            if replacement.exists() {
                std::fs::remove_dir_all(&replacement)?;
            }
            continue;
        }
        if rollback.exists() {
            if replacement.exists() {
                let _ = std::fs::remove_dir_all(&replacement);
            }
            std::fs::rename(&rollback, &live)?;
            continue;
        }
        if replacement.exists() {
            std::fs::rename(&replacement, &live)?;
        }
    }

    Ok(())
}
