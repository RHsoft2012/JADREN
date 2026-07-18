//! Deterministic MVP package manifest and lockfile support.
//!
//! The parser intentionally implements only the Jadren 0.1 manifest surface.
//! It does not execute build scripts, access a registry, or resolve versions;
//! those responsibilities belong to later package-security and resolver
//! milestones.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use jadren_determinism::{Fingerprint, fingerprint_bytes};

/// Canonical manifest filename.
pub const MANIFEST_FILE: &str = "jadren.toml";
/// Canonical generated lockfile filename.
pub const LOCKFILE_FILE: &str = "jadren.lock";
/// Current package manifest/lockfile schema.
pub const SCHEMA_VERSION: u32 = 1;

/// Package metadata declared by `[package]`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PackageMetadata {
    /// Registry/package identifier.
    pub name: String,
    /// Semantic package version.
    pub version: String,
    /// Language edition spelling.
    pub edition: String,
}

/// Build target switches declared by `[targets]`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TargetConfig {
    /// Whether the package exposes a library target.
    pub library: bool,
}

/// Parsed Jadren package manifest.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PackageManifest {
    /// Package metadata.
    pub package: PackageMetadata,
    /// Dependency name to specification, kept in stable key order.
    pub dependencies: BTreeMap<String, DependencySpec>,
    /// Target switches.
    pub targets: TargetConfig,
}

/// One manifest dependency declaration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DependencySpec {
    /// Registry/version requirement; resolution is a later gate.
    Version(String),
    /// Local package directory relative to the declaring manifest.
    Path(String),
}

impl DependencySpec {
    /// Returns the version requirement when this is a registry dependency.
    #[must_use]
    pub fn requirement(&self) -> Option<&str> {
        match self {
            Self::Version(value) => Some(value),
            Self::Path(_) => None,
        }
    }

    /// Returns the local path when this is a path dependency.
    #[must_use]
    pub fn path(&self) -> Option<&str> {
        match self {
            Self::Version(_) => None,
            Self::Path(value) => Some(value),
        }
    }
}

impl PackageManifest {
    /// Creates the canonical starter manifest for a package name.
    pub fn starter(name: impl Into<String>) -> Result<Self, ManifestError> {
        let name = name.into();
        validate_name(&name, 0)?;
        Ok(Self {
            package: PackageMetadata {
                name,
                version: "0.1.0".to_owned(),
                edition: "2026".to_owned(),
            },
            dependencies: BTreeMap::new(),
            targets: TargetConfig { library: true },
        })
    }

    /// Parses the supported TOML subset and rejects unknown/duplicate fields.
    pub fn parse(text: &str) -> Result<Self, ManifestError> {
        let mut section = None;
        let mut package_name = None;
        let mut package_version = None;
        let mut package_edition = None;
        let mut dependencies = BTreeMap::new();
        let mut library = None;

        for (index, raw_line) in text.lines().enumerate() {
            let line_number = index + 1;
            let line = strip_comment(raw_line).trim();
            if line.is_empty() {
                continue;
            }
            if line.starts_with('[') && line.ends_with(']') {
                let name = &line[1..line.len() - 1];
                if !matches!(name, "package" | "dependencies" | "targets") {
                    return Err(ManifestError::InvalidLine {
                        line: line_number,
                        message: format!("unsupported section `{name}`"),
                    });
                }
                section = Some(name.to_owned());
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                return Err(ManifestError::InvalidLine {
                    line: line_number,
                    message: "expected `key = value`".to_owned(),
                });
            };
            let key = key.trim();
            let value = value.trim();
            let Some(section_name) = section.as_deref() else {
                return Err(ManifestError::InvalidLine {
                    line: line_number,
                    message: "key appears before a section".to_owned(),
                });
            };
            match section_name {
                "package" => match key {
                    "name" => set_once(
                        &mut package_name,
                        parse_string(value, line_number)?,
                        key,
                        line_number,
                    )?,
                    "version" => set_once(
                        &mut package_version,
                        parse_string(value, line_number)?,
                        key,
                        line_number,
                    )?,
                    "edition" => set_once(
                        &mut package_edition,
                        parse_string(value, line_number)?,
                        key,
                        line_number,
                    )?,
                    _ => {
                        return Err(ManifestError::InvalidLine {
                            line: line_number,
                            message: format!("unsupported `[package]` key `{key}`"),
                        });
                    }
                },
                "dependencies" => {
                    validate_name(key, line_number)?;
                    if dependencies
                        .insert(key.to_owned(), parse_dependency(value, line_number)?)
                        .is_some()
                    {
                        return Err(ManifestError::DuplicateKey {
                            line: line_number,
                            key: key.to_owned(),
                        });
                    }
                }
                "targets" => {
                    if key != "library" {
                        return Err(ManifestError::InvalidLine {
                            line: line_number,
                            message: format!("unsupported `[targets]` key `{key}`"),
                        });
                    }
                    set_once(
                        &mut library,
                        parse_bool(value, line_number)?,
                        key,
                        line_number,
                    )?;
                }
                _ => unreachable!("section was validated above"),
            }
        }

        let name = package_name.ok_or(ManifestError::MissingKey {
            section: "package",
            key: "name",
        })?;
        validate_name(&name, 0)?;
        let version = package_version.ok_or(ManifestError::MissingKey {
            section: "package",
            key: "version",
        })?;
        validate_version(&version, 0)?;
        let edition = package_edition.ok_or(ManifestError::MissingKey {
            section: "package",
            key: "edition",
        })?;
        validate_edition(&edition, 0)?;

        for (dependency, specification) in &dependencies {
            if specification
                .requirement()
                .is_some_and(|requirement| requirement.trim().is_empty())
                || specification
                    .path()
                    .is_some_and(|path| path.trim().is_empty())
            {
                return Err(ManifestError::InvalidValue {
                    line: 0,
                    key: dependency.clone(),
                    message: "dependency requirement must not be empty".to_owned(),
                });
            }
        }

        Ok(Self {
            package: PackageMetadata {
                name,
                version,
                edition,
            },
            dependencies,
            targets: TargetConfig {
                library: library.unwrap_or(true),
            },
        })
    }

    /// Renders canonical TOML with stable section/key order.
    #[must_use]
    pub fn to_toml(&self) -> String {
        let mut output = String::new();
        output.push_str("[package]\n");
        let _ = writeln!(output, "name = {}", quote(&self.package.name));
        let _ = writeln!(output, "version = {}", quote(&self.package.version));
        let _ = writeln!(output, "edition = {}\n", quote(&self.package.edition));
        output.push_str("[dependencies]\n");
        for (name, specification) in &self.dependencies {
            match specification {
                DependencySpec::Version(requirement) => {
                    let _ = writeln!(output, "{name} = {}", quote(requirement));
                }
                DependencySpec::Path(path) => {
                    let _ = writeln!(output, "{name} = {{ path = {} }}", quote(path));
                }
            }
        }
        output.push_str("\n[targets]\n");
        let _ = writeln!(output, "library = {}", self.targets.library);
        output
    }

    /// Computes a stable identity over the canonical manifest text.
    #[must_use]
    pub fn fingerprint(&self) -> Fingerprint {
        fingerprint_bytes("jadren-package-manifest-v1", self.to_toml().as_bytes())
    }
}

/// One lockfile dependency entry. Resolution is intentionally explicit: MVP
/// records requirements but does not claim a registry or local resolver result.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LockedDependency {
    /// Dependency name.
    pub name: String,
    /// Manifest version requirement.
    pub requirement: String,
    /// Resolution status/source marker.
    pub source: String,
}

/// Deterministic generated lockfile.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PackageLockfile {
    /// Lockfile schema version.
    pub version: u32,
    /// Canonical manifest fingerprint.
    pub manifest_fingerprint: Fingerprint,
    /// Root package metadata.
    pub package: PackageMetadata,
    /// Dependency entries in stable name order.
    pub dependencies: Vec<LockedDependency>,
}

impl PackageLockfile {
    /// Creates an unresolved-but-auditable lockfile from a manifest.
    #[must_use]
    pub fn from_manifest(manifest: &PackageManifest) -> Self {
        Self {
            version: SCHEMA_VERSION,
            manifest_fingerprint: manifest.fingerprint(),
            package: manifest.package.clone(),
            dependencies: manifest
                .dependencies
                .iter()
                .map(|(name, specification)| LockedDependency {
                    name: name.clone(),
                    requirement: specification.requirement().unwrap_or("").to_owned(),
                    source: specification
                        .path()
                        .map_or_else(|| "unresolved".to_owned(), |path| format!("path:{path}")),
                })
                .collect(),
        }
    }

    /// Renders canonical lockfile text.
    #[must_use]
    pub fn to_toml(&self) -> String {
        let mut output = String::new();
        let _ = writeln!(output, "version = {}", self.version);
        let _ = writeln!(
            output,
            "manifest_fingerprint = {}",
            quote(&self.manifest_fingerprint.to_string())
        );
        output.push_str("\n[package]\n");
        let _ = writeln!(output, "name = {}", quote(&self.package.name));
        let _ = writeln!(output, "version = {}", quote(&self.package.version));
        let _ = writeln!(output, "edition = {}\n", quote(&self.package.edition));
        for dependency in &self.dependencies {
            output.push_str("[[dependency]]\n");
            let _ = writeln!(output, "name = {}", quote(&dependency.name));
            let _ = writeln!(output, "requirement = {}", quote(&dependency.requirement));
            let _ = writeln!(output, "source = {}\n", quote(&dependency.source));
        }
        output
    }
}

/// One package loaded by the local path resolver.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalPackage {
    /// Canonical manifest path used for identity and diagnostics.
    pub manifest_path: PathBuf,
    /// Parsed package manifest.
    pub manifest: PackageManifest,
}

/// Deterministic local dependency graph rooted at one manifest.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalResolution {
    /// Root package.
    pub root: LocalPackage,
    /// Transitive local dependencies in lexicographic package-name order.
    pub dependencies: Vec<LocalPackage>,
}

impl LocalResolution {
    /// Returns all packages, root first, for lockfile/report consumers.
    #[must_use]
    pub fn packages(&self) -> Vec<&LocalPackage> {
        let mut packages = Vec::with_capacity(self.dependencies.len() + 1);
        packages.push(&self.root);
        packages.extend(self.dependencies.iter());
        packages
    }
}

/// Failure while resolving local path dependencies.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ResolutionError {
    /// Manifest could not be read or parsed.
    Manifest { path: PathBuf, message: String },
    /// A version dependency needs the future registry resolver.
    UnresolvedVersion { package: String, dependency: String },
    /// A path dependency points to a package with a different declared name.
    NameMismatch {
        dependency: String,
        expected: String,
        actual: String,
        path: PathBuf,
    },
    /// A package name was encountered at two different paths.
    DuplicatePackage {
        name: String,
        first: PathBuf,
        second: PathBuf,
    },
    /// A dependency cycle was detected.
    Cycle { path: Vec<String> },
}

impl fmt::Display for ResolutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Manifest { path, message } => {
                write!(formatter, "manifest `{}`: {message}", path.display())
            }
            Self::UnresolvedVersion {
                package,
                dependency,
            } => write!(
                formatter,
                "package `{package}` has registry dependency `{dependency}`; local resolver requires a path dependency"
            ),
            Self::NameMismatch {
                dependency,
                expected,
                actual,
                path,
            } => write!(
                formatter,
                "dependency `{dependency}` expects package `{expected}` but `{}` declares `{actual}`",
                path.display()
            ),
            Self::DuplicatePackage {
                name,
                first,
                second,
            } => write!(
                formatter,
                "package `{name}` resolves to both `{}` and `{}`",
                first.display(),
                second.display()
            ),
            Self::Cycle { path } => {
                write!(formatter, "local dependency cycle: {}", path.join(" -> "))
            }
        }
    }
}

impl Error for ResolutionError {}

/// Resolves only relative local path dependencies without executing code or
/// accessing a registry. Paths are canonicalized for cycle/duplicate checks.
pub fn resolve_local(root: impl AsRef<Path>) -> Result<LocalResolution, ResolutionError> {
    let root_manifest =
        manifest_path(root.as_ref()).map_err(|message| ResolutionError::Manifest {
            path: root.as_ref().to_path_buf(),
            message,
        })?;
    let mut resolver = LocalResolver {
        packages: BTreeMap::new(),
        paths: BTreeMap::new(),
        visiting: Vec::new(),
    };
    let root_name = resolver.visit(&root_manifest, None)?;
    let root = resolver
        .packages
        .get(&root_name)
        .cloned()
        .expect("root package is inserted by visit");
    let dependencies = resolver
        .packages
        .into_iter()
        .filter_map(|(name, package)| (name != root_name).then_some(package))
        .collect();
    Ok(LocalResolution { root, dependencies })
}

struct LocalResolver {
    packages: BTreeMap<String, LocalPackage>,
    paths: BTreeMap<String, PathBuf>,
    visiting: Vec<String>,
}

impl LocalResolver {
    fn visit(
        &mut self,
        path: &Path,
        expected_name: Option<&str>,
    ) -> Result<String, ResolutionError> {
        let manifest_input = manifest_path(path).map_err(|message| ResolutionError::Manifest {
            path: path.to_path_buf(),
            message,
        })?;
        let manifest_path =
            fs::canonicalize(&manifest_input).map_err(|error| ResolutionError::Manifest {
                path: manifest_input.clone(),
                message: error.to_string(),
            })?;
        let text =
            fs::read_to_string(&manifest_path).map_err(|error| ResolutionError::Manifest {
                path: manifest_path.clone(),
                message: error.to_string(),
            })?;
        let manifest =
            PackageManifest::parse(&text).map_err(|error| ResolutionError::Manifest {
                path: manifest_path.clone(),
                message: error.to_string(),
            })?;
        let name = manifest.package.name.clone();
        if let Some(expected_name) = expected_name
            && expected_name != name
        {
            return Err(ResolutionError::NameMismatch {
                dependency: expected_name.to_owned(),
                expected: expected_name.to_owned(),
                actual: name,
                path: manifest_path,
            });
        }
        if let Some(index) = self.visiting.iter().position(|item| item == &name) {
            let mut cycle = self.visiting[index..].to_vec();
            cycle.push(name);
            return Err(ResolutionError::Cycle { path: cycle });
        }
        if let Some(existing_path) = self.paths.get(&name) {
            if existing_path != &manifest_path {
                return Err(ResolutionError::DuplicatePackage {
                    name,
                    first: existing_path.clone(),
                    second: manifest_path,
                });
            }
            return Ok(name);
        }
        self.visiting.push(name.clone());
        let package = LocalPackage {
            manifest_path: manifest_path.clone(),
            manifest: manifest.clone(),
        };
        self.paths.insert(name.clone(), manifest_path.clone());
        for (dependency_name, specification) in &manifest.dependencies {
            let DependencySpec::Path(relative_path) = specification else {
                return Err(ResolutionError::UnresolvedVersion {
                    package: name.clone(),
                    dependency: dependency_name.clone(),
                });
            };
            let dependency_path = manifest_path
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .join(relative_path);
            let _ = self.visit(&dependency_path, Some(dependency_name))?;
        }
        let popped = self.visiting.pop();
        debug_assert_eq!(popped.as_deref(), Some(name.as_str()));
        self.packages.insert(name.clone(), package);
        Ok(name)
    }
}

fn manifest_path(path: &Path) -> Result<PathBuf, String> {
    let metadata = fs::metadata(path).map_err(|error| error.to_string())?;
    if metadata.is_dir() {
        Ok(path.join(MANIFEST_FILE))
    } else if metadata.is_file() {
        Ok(path.to_path_buf())
    } else {
        Err("path is not a file or directory".to_owned())
    }
}

/// Manifest/lockfile parsing error.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ManifestError {
    /// A source line is not part of the supported schema.
    InvalidLine { line: usize, message: String },
    /// A key was declared twice.
    DuplicateKey { line: usize, key: String },
    /// A required field is absent.
    MissingKey {
        section: &'static str,
        key: &'static str,
    },
    /// A field has an invalid value.
    InvalidValue {
        line: usize,
        key: String,
        message: String,
    },
}

impl fmt::Display for ManifestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLine { line, message } => write!(formatter, "line {line}: {message}"),
            Self::DuplicateKey { line, key } => {
                write!(formatter, "line {line}: duplicate key `{key}`")
            }
            Self::MissingKey { section, key } => {
                write!(formatter, "missing `{section}.{key}`")
            }
            Self::InvalidValue { line, key, message } => {
                write!(formatter, "line {line}: invalid `{key}`: {message}")
            }
        }
    }
}

impl Error for ManifestError {}

fn set_once<T>(
    slot: &mut Option<T>,
    value: T,
    key: &str,
    line: usize,
) -> Result<(), ManifestError> {
    if slot.is_some() {
        return Err(ManifestError::DuplicateKey {
            line,
            key: key.to_owned(),
        });
    }
    *slot = Some(value);
    Ok(())
}

fn parse_string(value: &str, line: usize) -> Result<String, ManifestError> {
    if value.len() < 2 || !value.starts_with('"') || !value.ends_with('"') {
        return Err(ManifestError::InvalidValue {
            line,
            key: "string".to_owned(),
            message: "expected a double-quoted string".to_owned(),
        });
    }
    let value = &value[1..value.len() - 1];
    if value.contains('"') || value.contains('\n') || value.contains('\r') {
        return Err(ManifestError::InvalidValue {
            line,
            key: "string".to_owned(),
            message: "embedded quotes/newlines are not supported in MVP TOML".to_owned(),
        });
    }
    Ok(value.to_owned())
}

fn parse_dependency(value: &str, line: usize) -> Result<DependencySpec, ManifestError> {
    if value.starts_with('"') {
        return Ok(DependencySpec::Version(parse_string(value, line)?));
    }
    if !(value.starts_with('{') && value.ends_with('}')) {
        return Err(ManifestError::InvalidValue {
            line,
            key: "dependency".to_owned(),
            message: "expected a quoted version or `{ path = \"...\" }`".to_owned(),
        });
    }
    let body = value[1..value.len() - 1].trim();
    let Some((key, path_value)) = body.split_once('=') else {
        return Err(ManifestError::InvalidValue {
            line,
            key: "dependency".to_owned(),
            message: "path dependency requires `path = \"...\"`".to_owned(),
        });
    };
    if key.trim() != "path" {
        return Err(ManifestError::InvalidValue {
            line,
            key: "dependency".to_owned(),
            message: format!("unsupported inline dependency key `{}`", key.trim()),
        });
    }
    Ok(DependencySpec::Path(parse_string(path_value.trim(), line)?))
}

fn parse_bool(value: &str, line: usize) -> Result<bool, ManifestError> {
    match value {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(ManifestError::InvalidValue {
            line,
            key: "boolean".to_owned(),
            message: "expected `true` or `false`".to_owned(),
        }),
    }
}

fn validate_name(name: &str, line: usize) -> Result<(), ManifestError> {
    let mut chars = name.chars();
    let valid = chars
        .next()
        .is_some_and(|character| character.is_ascii_alphabetic() || character == '_')
        && chars
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-'));
    if valid {
        Ok(())
    } else {
        Err(ManifestError::InvalidValue {
            line,
            key: "name".to_owned(),
            message: format!("invalid package/dependency identifier `{name}`"),
        })
    }
}

fn validate_version(version: &str, line: usize) -> Result<(), ManifestError> {
    if version.split('.').count() == 3
        && version.split('.').all(|part| {
            !part.is_empty() && part.chars().all(|character| character.is_ascii_digit())
        })
    {
        Ok(())
    } else {
        Err(ManifestError::InvalidValue {
            line,
            key: "version".to_owned(),
            message: "expected semantic version `major.minor.patch`".to_owned(),
        })
    }
}

fn validate_edition(edition: &str, line: usize) -> Result<(), ManifestError> {
    if edition.len() == 4 && edition.chars().all(|character| character.is_ascii_digit()) {
        Ok(())
    } else {
        Err(ManifestError::InvalidValue {
            line,
            key: "edition".to_owned(),
            message: "expected four-digit edition".to_owned(),
        })
    }
}

fn strip_comment(line: &str) -> &str {
    let mut in_string = false;
    for (index, character) in line.char_indices() {
        if character == '"' {
            in_string = !in_string;
        } else if character == '#' && !in_string {
            return &line[..index];
        }
    }
    line
}

fn quote(value: &str) -> String {
    let mut output = String::from("\"");
    for character in value.chars() {
        match character {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            character => output.push(character),
        }
    }
    output.push('"');
    output
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{DependencySpec, PackageLockfile, PackageManifest, SCHEMA_VERSION, resolve_local};

    #[test]
    fn parses_and_renders_canonical_manifest() {
        let manifest = PackageManifest::parse(
            "# comment\n[targets]\nlibrary = true\n\n[dependencies]\nzeta = \"0.2\"\nmath = \"0.1\"\n\n[package]\nedition = \"2026\"\nversion = \"0.1.0\"\nname = \"demo\"\n",
        )
        .expect("valid manifest");
        assert_eq!(
            manifest.dependencies.keys().collect::<Vec<_>>(),
            vec![&"math".to_owned(), &"zeta".to_owned()]
        );
        assert!(manifest.to_toml().contains("math = \"0.1\""));
        assert_eq!(
            manifest,
            PackageManifest::parse(&manifest.to_toml()).unwrap()
        );
    }

    #[test]
    fn lockfile_is_deterministic_and_explicitly_unresolved() {
        let manifest = PackageManifest::starter("demo").expect("starter");
        let lock = PackageLockfile::from_manifest(&manifest);
        assert_eq!(lock.version, SCHEMA_VERSION);
        assert!(lock.to_toml().contains("manifest_fingerprint"));
        assert!(lock.dependencies.is_empty());
    }

    #[test]
    fn rejects_duplicate_and_invalid_fields() {
        let duplicate = "[package]\nname = \"demo\"\nname = \"other\"\nversion = \"0.1.0\"\nedition = \"2026\"\n";
        assert!(PackageManifest::parse(duplicate).is_err());
        let invalid = "[package]\nname = \"demo\"\nversion = \"0.1\"\nedition = \"2026\"\n";
        assert!(PackageManifest::parse(invalid).is_err());
    }

    #[test]
    fn resolves_local_path_dependencies_in_stable_order() {
        let root = std::env::temp_dir().join(format!("jadren-package-test-{}", std::process::id()));
        let dependency = root.join("math");
        fs::create_dir_all(&dependency).expect("dependency directory");
        fs::write(
            root.join("jadren.toml"),
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2026\"\n[dependencies]\nmath = { path = \"math\" }\n",
        )
        .expect("root manifest");
        fs::write(
            dependency.join("jadren.toml"),
            "[package]\nname = \"math\"\nversion = \"0.1.0\"\nedition = \"2026\"\n",
        )
        .expect("dependency manifest");
        let manifest =
            PackageManifest::parse(&fs::read_to_string(root.join("jadren.toml")).unwrap())
                .expect("path manifest");
        assert_eq!(
            manifest.dependencies["math"],
            DependencySpec::Path("math".to_owned())
        );
        let resolution = resolve_local(&root).expect("local resolution");
        assert_eq!(resolution.root.manifest.package.name, "app");
        assert_eq!(resolution.dependencies.len(), 1);
        assert_eq!(resolution.dependencies[0].manifest.package.name, "math");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn local_resolver_rejects_registry_dependency() {
        let root =
            std::env::temp_dir().join(format!("jadren-package-registry-{}", std::process::id()));
        fs::create_dir_all(&root).expect("root directory");
        fs::write(
            root.join("jadren.toml"),
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2026\"\n[dependencies]\nmath = \"0.1\"\n",
        )
        .expect("root manifest");
        assert!(resolve_local(&root).is_err());
        let _ = fs::remove_dir_all(root);
    }
}
