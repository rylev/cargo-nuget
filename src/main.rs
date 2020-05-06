use cargo_toml::{Manifest, Value};
use structopt::StructOpt;

fn main() {
    let opt = Opt::from_args();
    match opt.subcommand {
        Subcommand::Install(i) => do_install().unwrap(),
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

#[derive(Debug, StructOpt)]
pub struct Install {}

fn do_install() -> std::io::Result<()> {
    let bytes = std::fs::read("Cargo.toml")?;
    let manifest = Manifest::from_slice(&bytes).unwrap();
    // println!("{:#?}", manifest);
    let deps = get_deps(manifest);
    println!("{:#?}", deps);
    download_dependencies(deps);

    Ok(())
}

fn get_deps(manifest: Manifest) -> Vec<Dependency> {
    let metadata = manifest.package.unwrap().metadata.unwrap();
    match metadata {
        Value::Table(mut t) => {
            let deps = match t.remove("nuget_dependencies") {
                Some(Value::Table(deps)) => deps,
                _ => panic!("Not there"),
            };
            deps.into_iter()
                .map(|(key, value)| {
                    let version = match value {
                        Value::String(version) => version,
                        _ => panic!("Version was not a string"),
                    };
                    Dependency { name: key, version }
                })
                .collect()
        }
        _ => panic!("Ain't no table"),
    }
}

#[derive(Debug)]
struct Dependency {
    name: String,
    version: String,
}
impl Dependency {
    fn url(&self) -> String {
        format!(
            "https://www.nuget.org/api/v2/package/{}/{}",
            self.name, self.version
        )
    }
}

use async_std::task;

type Error = Box<dyn std::error::Error + std::marker::Send + std::marker::Sync>;
fn download_dependencies(deps: Vec<Dependency>) -> Vec<(Dependency, Vec<u8>)> {
    task::block_on(async {
        let results = deps.into_iter().map(|dep| async move {
            let mut res = surf::get(dep.url()).await?;
            let bytes = res.body_bytes().await?;
            Ok::<_, Error>((dep, bytes))
        });

        let results: Result<Vec<_>, Error> = futures::future::try_join_all(results).await;

        results
    })
    .unwrap()
}
