use async_std::task;
use cargo_toml::{Manifest, Value};
use structopt::StructOpt;
use thiserror::Error;

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
}

#[derive(Debug, StructOpt)]
pub struct Install {}

impl Install {
    fn perform(&self) -> Result<(), Error> {
        let bytes = std::fs::read("Cargo.toml").map_err(|_| Error::NoCargoToml)?;
        let manifest = Manifest::from_slice(&bytes).map_err(|_| Error::MalformedManifest)?;
        let deps = get_deps(manifest)?;
        download_dependencies(deps)?;

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
        let mut res = surf::get(self.url())
            .await
            .map_err(|e| Error::DownloadError(e))?;
        let bytes = res
            .body_bytes()
            .await
            .map_err(|e| Error::DownloadError(e.into()))?;
        Ok(bytes)
    }
}

fn download_dependencies(deps: Vec<Dependency>) -> Result<Vec<(Dependency, Vec<u8>)>, Error> {
    task::block_on(async {
        let results = deps.into_iter().map(|dep| async move {
            let bytes = dep.download().await?;
            Ok((dep, bytes))
        });

        futures::future::try_join_all(results).await
    })
}
