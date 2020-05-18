use cargo_toml::{Manifest, Value};
use futures::future::{BoxFuture, FutureExt};
use structopt::StructOpt;
use thiserror::Error;

use std::ffi::OsString;
use std::io::Read;
use std::path::{Path, PathBuf};

fn main() {
    let opt = Opt::from_args();
    match opt.subcommand {
        Subcommand::Install(i) => i.perform().unwrap(),
    }
}

/// A utility for interacting with nuget packages
#[derive(StructOpt, Debug)]
#[structopt(name = "nuget")]
struct Opt {
    #[structopt(subcommand)]
    pub subcommand: Subcommand,
}

#[derive(Debug, StructOpt)]
enum Subcommand {
    Install(Install),
}

#[derive(Debug, Error)]
enum Error {
    #[error("No Cargo.toml could be found")]
    NoCargoToml,
    #[error("There was an error downloading the NuGet package {0}")]
    DownloadError(Box<dyn std::error::Error>),
    #[error("The Cargo.toml file was malformed")]
    MalformedManifest,
    #[error("There was some other error {0}")]
    Other(Box<dyn std::error::Error>),
}

#[derive(Debug, StructOpt)]
pub struct Install {}

impl Install {
    fn perform(&self) -> Result<(), Error> {
        let bytes = std::fs::read("Cargo.toml").map_err(|_| Error::NoCargoToml)?;
        let manifest = Manifest::from_slice(&bytes).map_err(|_| Error::MalformedManifest)?;
        let deps = get_deps(manifest)?;
        let downloaded_deps = download_dependencies(deps)?;
        for dep in downloaded_deps {
            let winmds = dep.winmds()?;
            let dep_directory = PathBuf::new()
                .join("target")
                .join("nuget")
                .join(dep.dependency.name);
            // create the dependency directory
            std::fs::create_dir_all(&dep_directory).unwrap();
            for winmd in winmds {
                winmd.write(&dep_directory).unwrap();
            }
        }

        Ok(())
    }
}

fn get_deps(manifest: Manifest) -> Result<Vec<Dependency>, Error> {
    let metadata = manifest.package.and_then(|p| p.metadata);
    match metadata {
        Some(Value::Table(mut t)) => {
            let deps = match t.remove("nuget_dependencies") {
                Some(Value::Table(deps)) => deps,
                _ => return Err(Error::MalformedManifest.into()),
            };
            deps.into_iter()
                .map(|(key, value)| match value {
                    Value::String(version) => Ok(Dependency::new(key, version)),
                    _ => Err(Error::MalformedManifest.into()),
                })
                .collect()
        }
        _ => return Err(Error::MalformedManifest.into()),
    }
}

#[derive(Debug)]
struct Dependency {
    name: String,
    version: String,
}

impl Dependency {
    fn new(name: String, version: String) -> Self {
        Self { name, version }
    }

    fn url(&self) -> String {
        format!(
            "https://www.nuget.org/api/v2/package/{}/{}",
            self.name, self.version
        )
    }

    async fn download(&self) -> Result<Vec<u8>, Error> {
        fn try_download(
            url: String,
            recursion_amount: u8,
        ) -> BoxFuture<'static, Result<Vec<u8>, Error>> {
            async move {
                if recursion_amount == 0 {
                    return Err(Error::DownloadError(
                        anyhow::anyhow!("Too many redirects").into(),
                    ));
                }
                let res = reqwest::get(&url)
                    .await
                    .map_err(|e| Error::DownloadError(e.into()))?;
                match res.status().into() {
                    200u16 => {
                        let bytes = res
                            .bytes()
                            .await
                            .map_err(|e| Error::DownloadError(e.into()))?;
                        Ok(bytes.into_iter().collect())
                    }
                    302 => {
                        let headers = res.headers();
                        let redirect_url = headers.get("Location").unwrap();

                        let url = redirect_url.to_str().unwrap();

                        try_download(url.to_owned(), recursion_amount - 1).await
                    }
                    _ => {
                        return Err(Error::DownloadError(
                            anyhow::anyhow!("Non-successful response: {}", res.status()).into(),
                        ))
                    }
                }
            }
            .boxed()
        }

        try_download(self.url(), 5).await
    }
}

struct DownloadedDependency {
    dependency: Dependency,
    bytes: Vec<u8>,
}

impl DownloadedDependency {
    fn winmds(&self) -> Result<Vec<Winmd>, Error> {
        let reader = std::io::Cursor::new(&self.bytes);
        let mut zip = zip::ZipArchive::new(reader).map_err(|e| Error::Other(Box::new(e)))?;
        let mut winmds = Vec::new();
        for i in 0..zip.len() {
            let mut file = zip.by_index(i).unwrap();
            let path = file.sanitized_name();
            match path.extension() {
                Some(e)
                    if e == "winmd"
                        && path.parent().and_then(Path::to_str) == Some("lib\\uap10.0") =>
                {
                    let name = path.file_name().unwrap().to_owned();
                    let mut contents = Vec::with_capacity(file.size() as usize);

                    if let Err(e) = file.read_to_end(&mut contents) {
                        eprintln!("Could not read winmd file: {:?}", e);
                        continue;
                    }
                    winmds.push(Winmd { name, contents });
                }
                _ => {}
            }
        }
        Ok(winmds)
    }
}

fn download_dependencies(deps: Vec<Dependency>) -> Result<Vec<DownloadedDependency>, Error> {
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let results = deps.into_iter().map(|dep| async move {
            let bytes = dep.download().await?;
            Ok(DownloadedDependency {
                dependency: dep,
                bytes,
            })
        });

        futures::future::try_join_all(results).await
    })
}

struct Winmd {
    name: OsString,
    contents: Vec<u8>,
}

impl Winmd {
    fn write(&self, dir: &Path) -> std::io::Result<()> {
        std::fs::write(dir.join(&self.name), &self.contents)
    }
}
