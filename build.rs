use std::{collections::HashMap, path::PathBuf};

use cargo_metadata::DependencyKind;

// Compress the assets from game + mods into an archive that is included in the executable.
fn main() {
    println!("cargo:rerun-if-changed=assets");

    let mut asset_paths = HashMap::new();
    for asset_path in get_asset_paths() {
        for asset in walk_dir(asset_path.join("assets/client")) {
            let relative_asset_path = asset.strip_prefix(&asset_path).unwrap().to_path_buf();
            asset_paths.insert(relative_asset_path, asset);
        }

        for asset in walk_dir(asset_path.join("assets/server")) {
            let relative_asset_path = asset.strip_prefix(&asset_path).unwrap().to_path_buf();
            asset_paths.insert(relative_asset_path, asset);
        }
    }

    let mut archive = tar::Builder::new(Vec::new());
    for (relative_path, absolute_path) in asset_paths {
        archive
            .append_path_with_name(absolute_path, relative_path)
            .unwrap();
    }

    let out_dir = std::env::var("OUT_DIR").unwrap();
    let compressed: Vec<u8> =
        zstd::stream::encode_all(archive.into_inner().unwrap().as_slice(), 19).unwrap();
    let dest_path = std::path::Path::new(&out_dir).join("assets.tar.zstd");

    std::fs::write(dest_path, compressed).unwrap();
}

fn walk_dir<P: AsRef<std::path::Path>>(dir: P) -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();

    let Ok(directory) = std::fs::read_dir(&dir) else {
        return files;
    };

    for entry in directory {
        let file_path = entry.unwrap().path();

        if file_path.is_dir() {
            let sub_files = walk_dir(&file_path);
            files.extend(sub_files);
        } else {
            files.push(file_path);
        }
    }

    files
}

fn get_asset_paths() -> Vec<PathBuf> {
    let manifest_path = std::env::var_os("CARGO_MANIFEST_PATH").unwrap();
    let meta = cargo_metadata::MetadataCommand::new()
        .manifest_path(&manifest_path)
        .exec()
        .unwrap();

    // Linear search over 'packages' with string eq for each dependency would get expensive with many mods.
    // Pre-compute a hashmap for faster index lookup.
    let mut index_lookup = HashMap::with_capacity(meta.packages.len());
    for (index, package) in meta.packages.iter().enumerate() {
        index_lookup.insert(package.name.clone(), index);
    }

    // The dependencies returned in metadata are in alphabetical order.
    // The dependency order in Cargo.toml is the asset priority order, so we
    // have to build it independently. The 'toml' crate has a 'preserve order' feature we use.
    let mut asset_paths_unsorted = HashMap::new();

    let root_package = meta.root_package().unwrap();
    for dependency in root_package.dependencies.iter() {
        if dependency.kind != DependencyKind::Normal {
            continue;
        }

        let package = &meta.packages[index_lookup[&dependency.name]];

        asset_paths_unsorted.insert(
            package.name.clone(),
            PathBuf::from(package.manifest_path.parent().unwrap()),
        );
    }

    let mut asset_paths = Vec::new();

    // TODO: Currently using the order dependencies appear in Cargo.toml to decide asset
    // presedence. This should not be left to the user. Sort mod assets into their own separate
    // directories instead of overwriting. Make some resolution mechanism at runtime the
    // mods can hook into to prioritize themselves.
    let manifest =
        toml::from_str::<toml::Table>(&std::fs::read_to_string(&manifest_path).unwrap()).unwrap();
    for (name, data) in manifest
        .get("dependencies")
        .and_then(|t| t.as_table())
        .into_iter()
        .flatten()
    {
        // Dependencies can be renamed by specifying the crate name in the 'package' field
        let name = data.get("package").and_then(|p| p.as_str()).unwrap_or(name);

        asset_paths.push(asset_paths_unsorted.remove(name).unwrap());
    }

    // Include assets from the binary's directory. For when you build the the library
    // crate as a binary or are creating a mod, where they wouldn't be counted among the
    // dependencies.
    asset_paths.push(std::env::var_os("CARGO_MANIFEST_DIR").unwrap().into());

    return asset_paths;
}
