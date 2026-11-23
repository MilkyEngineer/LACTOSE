//! A directory of sources cooked into one pack (`README.md`, "The command line").

use std::collections::BTreeMap;
use std::path::Path;

use lactose_asset::{PackBuilder, Storage};
use lactose_asset_api::{AssetId, AssetKind, AssetVersion, CookedAsset, CookedHeader, Processor};
use lactose_core::{Error, ErrorKind, Result};
use lactose_io::{EntryKind, Io};
use lactose_rhi_api::COOKED_SHADER_VERSION;

use crate::shader::{self, LINK, MODULES, Slang, Source};
use crate::skeleton::SkeletonProcessor;

/// The file name ending of a source the cooker cooks to a skeleton.
const SOURCE: &str = ".gltf";
/// The file name ending of a binary glTF, which the cooker refuses.
const BINARY: &str = ".glb";
/// The file name ending of a Slang source: a shader, a link module or an imported module.
const SLANG: &str = ".slang";
/// The kind of a cooked shader, whose bytes are `CookedShader::encode`'s and whose format
/// version is `COOKED_SHADER_VERSION`.
pub const SHADER_KIND: AssetKind = AssetKind::from_name("rhi::shader");
/// The ending of the asset name of a cooked skeleton, in place of [`SOURCE`].
const SKELETON: &str = ".skeleton";
/// The file name ending of a pack: the one that places a file in the cooked pack set
/// (`lactose_asset::pack_set`).
const PACK: &str = ".pack";
/// How every asset is stored: zstd at a fixed level, so the bytes are a function of the
/// sources and of the build alone.
const STORAGE: Storage = Storage::Zstd { level: 3 };

/// The shader toolchain a cook runs, and where it also writes each shader's code.
#[derive(Clone, Copy, Debug)]
pub struct Shaders<'a> {
    /// The pinned `slangc`, checked.
    pub slang: &'a Slang,
    /// A directory that also receives each cooked shader's code alone, the SPIR-V module,
    /// the MSL or the metallib, as the file `<directory>/<asset name>`; or `None`.
    pub code: Option<&'a Path>,
    /// A directory that also receives each cooked shader whole, as `CookedShader::encode`
    /// wrote it and `CookedShader::decode` reads it, as the file `<directory>/<asset name>`;
    /// or `None`.
    pub cooked: Option<&'a Path>,
}

/// One asset a cook wrote: its name, the ID that name hashes to (`AssetId::from_name`) and
/// the version the pack holds.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Published {
    /// The asset name: the source's path below the sources directory, `/`-separated, its
    /// `.gltf` ending replaced by `.skeleton`, or for a shader its `.slang` ending by
    /// `.<entry>.<target>` (`README.md`, "The shader kind").
    pub name: String,
    /// `AssetId::from_name(&name)`.
    pub id: AssetId,
    /// The content hash of the cooked asset (`CookedHeader::version_of`).
    pub version: AssetVersion,
}

/// Cooks every `.gltf` file under `sources`, at any depth, to a skeleton, and every shader
/// source with `shaders`' toolchain to a shader for each entry point and target, and writes
/// them all as one pack at `pack`, replacing the file by rename and creating its directory.
/// Returns what it wrote, in byte order of the names.
///
/// Every file request goes through `io`, so the future is driven by `Executor::block_on` or
/// awaited on an executor; the shader toolchain runs as subprocesses on the thread that
/// polls it, reading the sources itself. Nothing is written unless every source cooks; the
/// code files of [`Shaders::code`] are written before the pack. Fails with:
/// `InvalidData` if `pack`'s file name does not end in `.pack`, for a file or directory
/// name below `sources` that is not UTF-8, for a source the processor or the shader checks
/// refuse (the message starts with the source's path below `sources`, then names the node
/// or the entry point), for a link module with no shader beside it, and for two names
/// that hash to one ID or a name that hashes to `AssetId::NONE`; `Unsupported` for a
/// `.glb` file or an entry that is neither a file nor a directory (a symbolic link among
/// them); `NotFound` if `sources` holds no `.gltf` and no shader source, or a shader source
/// and `shaders` is `None`; and whatever `io` reports.
pub async fn cook(
    io: &Io,
    sources: &Path,
    pack: &Path,
    shaders: Option<Shaders<'_>>,
) -> Result<Vec<Published>> {
    if !pack.file_name().and_then(|name| name.to_str()).is_some_and(|name| name.ends_with(PACK)) {
        return Err(Error::new(
            ErrorKind::InvalidData,
            format!("the pack's file name ends in `{PACK}`, which {} does not", pack.display()),
        ));
    }
    let listed = list(io, sources).await?;
    if listed.gltf.is_empty() && listed.shaders.is_empty() {
        return Err(Error::new(
            ErrorKind::NotFound,
            format!("no `{SOURCE}` or `{SLANG}` file under {}", sources.display()),
        ));
    }
    let mut assets = Vec::new();
    for path in listed.gltf {
        let bytes = io.read_file(sources.join(&path)).await.map_err(at(&path))?;
        let cooked = SkeletonProcessor.process(&bytes).map_err(at(&path))?;
        let name = format!("{}{SKELETON}", path.strip_suffix(SOURCE).expect("a source's name"));
        assets.push((name, cooked, None));
    }
    if let Some(first) = listed.shaders.first() {
        let Some(shaders) = shaders else {
            return Err(Error::new(
                ErrorKind::NotFound,
                format!(
                    "{}: a shader source, and the cook was given no slangc: `cargo xtask slang` \
                     installs the pinned release",
                    first.path
                ),
            ));
        };
        for source in &listed.shaders {
            for cooked in shader::cook(shaders.slang, sources, source)? {
                let bytes = cooked.shader.encode().map_err(at(&cooked.name))?;
                let whole = shaders.cooked.map(|_| bytes.clone());
                let asset = CookedAsset {
                    header: CookedHeader::new(SHADER_KIND, COOKED_SHADER_VERSION, []),
                    bytes,
                };
                assets.push((cooked.name, asset, Some((cooked.shader.code, whole))));
            }
        }
    }
    assets.sort_unstable_by(|a, b| a.0.cmp(&b.0));
    let mut claimed = BTreeMap::new();
    let mut builder = PackBuilder::new();
    let mut published = Vec::with_capacity(assets.len());
    let mut code = Vec::new();
    for (name, cooked, shader_code) in assets {
        let id = claim(&mut claimed, &name)?;
        let version = builder.add(id, &cooked, STORAGE)?;
        if let Some((shader_code, whole)) = shader_code {
            code.push((name.clone(), shader_code, whole));
        }
        published.push(Published { name, id, version });
    }
    let bytes = builder.finish()?;
    let (code_dir, cooked_dir) =
        shaders.map_or((None, None), |shaders| (shaders.code, shaders.cooked));
    for (name, shader_code, whole) in code {
        let written = [(code_dir, Some(shader_code)), (cooked_dir, whole)];
        for (directory, bytes) in written {
            let (Some(directory), Some(bytes)) = (directory, bytes) else { continue };
            let file = directory.join(&name);
            let parent = file.parent().expect("a file below the directory");
            io.create_dir_all(parent).await.map_err(at(parent.display()))?;
            io.replace(&file, bytes).await.map_err(|failed| at(file.display())(failed.error))?;
        }
    }
    if let Some(directory) = pack.parent().filter(|parent| !parent.as_os_str().is_empty()) {
        io.create_dir_all(directory).await.map_err(at(directory.display()))?;
    }
    io.replace(pack, bytes).await.map_err(|failed| at(pack.display())(failed.error))?;
    Ok(published)
}

/// The sources a cook reads, each list in byte order of the paths below `sources`.
struct Listed {
    /// Each `.gltf` file.
    gltf: Vec<String>,
    /// Each shader source and its link module.
    shaders: Vec<Source>,
}

/// The sources below `sources`, their paths `/`-separated, in byte order: the order the
/// cook reads them in, whatever order the platform lists a directory in.
async fn list(io: &Io, sources: &Path) -> Result<Listed> {
    let mut gltf = Vec::new();
    let mut slang = Vec::new();
    let mut directories = vec![String::new()];
    while let Some(directory) = directories.pop() {
        let listed = sources.join(&directory);
        for entry in io.read_dir(&listed).await.map_err(at(listed.display()))? {
            let Some(name) = entry.name.to_str() else {
                return Err(Error::new(
                    ErrorKind::InvalidData,
                    format!("{directory}{}: a name that is not UTF-8", entry.name.display()),
                ));
            };
            let path = format!("{directory}{name}");
            match entry.kind {
                EntryKind::Directory => directories.push(format!("{path}/")),
                EntryKind::File if only_an_ending(name) => {
                    return Err(Error::new(
                        ErrorKind::InvalidData,
                        format!(
                            "{path}: a name that is only an ending, which would cook to an asset \
                             whose file name is empty: give it a name before the ending"
                        ),
                    ));
                }
                EntryKind::File if let Some(renamed) = lowercase_ending(name) => {
                    return Err(Error::new(
                        ErrorKind::InvalidData,
                        format!(
                            "{path}: an ending the cooker reads in lowercase alone, so that an \
                             asset's name is the same on every file system: rename it `{renamed}`"
                        ),
                    ));
                }
                EntryKind::File if name.ends_with(SOURCE) => gltf.push(path),
                EntryKind::File if name.ends_with(BINARY) => {
                    return Err(Error::new(
                        ErrorKind::Unsupported,
                        format!("{path}: a binary glTF, where the cooker reads `{SOURCE}`"),
                    ));
                }
                EntryKind::File if name.ends_with(SLANG) && !path.starts_with(MODULES) => {
                    slang.push(path);
                }
                EntryKind::File => {}
                EntryKind::Other => {
                    return Err(Error::new(
                        ErrorKind::Unsupported,
                        format!("{path}: neither a file nor a directory"),
                    ));
                }
            }
        }
    }
    gltf.sort_unstable();
    let (links, shaders): (Vec<String>, Vec<String>) =
        slang.into_iter().partition(|path| path.ends_with(LINK));
    for link in &links {
        let shader = format!("{}{SLANG}", link.strip_suffix(LINK).expect("a link module"));
        if !shaders.contains(&shader) {
            return Err(Error::new(
                ErrorKind::InvalidData,
                format!("{link}: a link module, and there is no shader `{shader}` beside it"),
            ));
        }
    }
    let mut shaders: Vec<Source> = shaders
        .into_iter()
        .map(|path| {
            let link = format!("{}{LINK}", path.strip_suffix(SLANG).expect("a shader"));
            Source { link: links.contains(&link).then_some(link), path }
        })
        .collect();
    shaders.sort_unstable_by(|a, b| a.path.cmp(&b.path));
    Ok(Listed { gltf, shaders })
}

/// Whether `name` is `.gltf`, `.glb` or `.slang` alone, in any case.
fn only_an_ending(name: &str) -> bool {
    [SOURCE, BINARY, SLANG].iter().any(|ending| name.eq_ignore_ascii_case(ending))
}

/// `name` with its ending in lowercase, if it ends in `.gltf`, `.glb` or `.slang` written in
/// another case (`.GLTF`, `.Glb`, `.Slang`).
fn lowercase_ending(name: &str) -> Option<String> {
    let (stem, ending) = name.rsplit_once('.')?;
    let lower = ending.to_ascii_lowercase();
    (lower != ending && [SOURCE, BINARY, SLANG].contains(&format!(".{lower}").as_str()))
        .then(|| format!("{stem}.{lower}"))
}

/// The error of a request or a processor on `what`, a path, prefixed by it: the pool's
/// errors name no path.
fn at(what: impl core::fmt::Display) -> impl FnOnce(Error) -> Error {
    move |error| Error::new(error.kind(), format!("{what}: {}", error.message()))
}

/// The ID of `name`, claimed for it in `claimed`: refused if another name of the cook has
/// that ID, or if it is `AssetId::NONE`, which names no asset.
fn claim(claimed: &mut BTreeMap<AssetId, String>, name: &str) -> Result<AssetId> {
    let id = AssetId::from_name(name);
    if id.is_none() {
        return Err(Error::new(ErrorKind::InvalidData, format!("{name}: its ID is the empty ID")));
    }
    if let Some(other) = claimed.insert(id, name.to_owned()) {
        return Err(Error::new(
            ErrorKind::InvalidData,
            format!("{name}: its ID {:#018x} is also {other}'s", id.to_bits()),
        ));
    }
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_is_claimed_once() {
        let mut claimed = BTreeMap::new();
        let id = claim(&mut claimed, "a.skeleton").unwrap();
        assert_eq!(id, AssetId::from_name("a.skeleton"));
        assert_eq!(claim(&mut claimed, "b.skeleton").unwrap(), AssetId::from_name("b.skeleton"));
        let error = claim(&mut claimed, "a.skeleton").unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidData);
        assert!(error.message().contains("also a.skeleton's"), "{error}");
    }
}
