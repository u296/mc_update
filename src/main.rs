extern crate app_dirs;
extern crate clap;
extern crate regex;
extern crate reqwest;
extern crate select;

use std::env;
use std::fs;
use std::io;
use std::path::Path;
use std::process::exit;

use clap::{App, Arg};
use regex::Regex;
use select::document::Document;
use select::predicate::Name;

const APP_INFO: app_dirs::AppInfo = app_dirs::AppInfo { name: "mc_update", author: "u296" };

const LOCAL_REPO_PATH: &str = "./mc_update";

fn download(url: &str, max_tries: usize) -> Option<reqwest::blocking::Response> {
    for i in 1..=max_tries {
        match reqwest::blocking::get(url) {
            Ok(response) => {
                println!("info : ({}/{}) get request succeeded for url: {}", i, max_tries, url);
                return Some(response);
            }
            Err(e) => {
                if let Some(code) = e.status() {
                    println!("warning ({}/{}): get request failed with code '{}': {}", i, max_tries, code.as_u16(), code.canonical_reason().unwrap());
                } else {
                    println!("error: get request failed for other reason");
                    return None;
                }
            }
        }
        if i == max_tries {
            println!("error: {}/{} attempts at downloading file failed", max_tries, max_tries);
            return None;
        }
    };
    None
}

fn create_file(path: &Path) -> Option<fs::File> {
    match fs::File::create(&path) {
        Ok(f) => Some(f),
        Err(e) => {
            println!("warning: failed to create file '{}': {}", path.display(), e);
            None
        }
    }
}

fn main() {
    let matches = App::new("mc_update")
        .version(env!("CARGO_PKG_VERSION"))
        .about("Updates minecraft server jars")
        .arg(Arg::with_name("repo")
            .short("r")
            .long("repository")
            .help("adds a custom repository")
            .takes_value(true)
            .multiple(true))
        .arg(Arg::with_name("install_repo")
            .short("i")
            .long("install_repository")
            .help("adds a repository to download to if needed, e.g. './repo/jars'")
            .takes_value(true)
            .multiple(true))
        .arg(Arg::with_name("JAR_VERSION")
            .help("the jar version to update to, e.g. '1.12.2'")
            .required(true)
            .index(1))
        .get_matches();

    let jar_version = matches.value_of("JAR_VERSION").unwrap().to_string();

    let r = Regex::new(r#"(\.\.)|/|\\"#).unwrap();

    if r.is_match(jar_version.as_str()) {
        println!("error: jar version contains invalid characters");
        exit(1);
    }

    let global_repo_path = app_dirs::get_app_root(app_dirs::AppDataType::UserCache, &APP_INFO)
        .expect("error: failed to create global cache dir");

    let global_repo_path = global_repo_path.to_str()
        .unwrap();

    let repos = {
        let global_repo = Path::new(global_repo_path);
        let local_repo = Path::new(LOCAL_REPO_PATH);

        let mut repos: Vec<&Path> = Vec::new();

        for repo in [global_repo, local_repo].iter() {
            if repo.is_dir() {
                repos.push(repo);
            }
        }

        for repo in matches.values_of("repo").unwrap_or_default().chain(matches.values_of("install_repo").unwrap_or_default()) {
            let repo = Path::new(repo);
            if repo.exists() {
                if !repo.is_dir() {
                    repos.push(repo);
                }
                println!("warning: '{}' is not a directory, ignoring", repo.display());
            } else {
                println!("warning: the repo '{}' does not exist", repo.display());
            }
        }
        repos
    };
    let mut jar_file = None;

    'out: for repo in repos.iter() {
        for file in repo.read_dir().unwrap() {
            let file = file.unwrap();
            if file.path().is_file() && file.file_name().into_string().unwrap() == jar_version {
                jar_file = Some(file.path());
                break 'out;
            }
        }
    }

    if jar_file.is_none() {
        let mut install_repo: Option<&Path> = None;

        for repo in matches.values_of("install_repo").unwrap_or_default().chain([global_repo_path, LOCAL_REPO_PATH].iter().copied()) {
            let repo = Path::new(repo);
            if repo.exists() {
                if !repo.is_dir() {
                    println!("warning: '{}' is not a directory, ignoring", repo.display());
                    continue;
                }
                install_repo = Some(repo);
                break;
            } else {
                match fs::create_dir_all(repo) {
                    Ok(_) => {
                        println!("info: created repo at '{}'", repo.display());
                        install_repo = Some(repo);
                        break;
                    }
                    Err(e) => {
                        println!("warning: failed to create repo at '{}': '{}'\ncontinue? (Y/n)", repo.display(), e);
                        let mut input = String::new();
                        io::stdin().read_line(&mut input).expect("error: failed to read stdin");
                        input.to_ascii_lowercase();
                        if input.contains("n") {
                            exit(0);
                        }
                    }
                }
            }
        }

        let install_repo = install_repo.unwrap();

        let target = format!("https://mcversions.net/download/{}", jar_version);

        let page = match download(target.as_str(), 3) {
            Some(x) => x.text().unwrap(),
            _ => exit(1),
        };


        let server_jar_url = match Document::from(page.as_str()).find(Name("a"))
            .filter_map(|n| { if n.attr("href")?.contains("server.jar") { Some(n.attr("href")) } else { None } })
            .collect::<Vec<_>>().get(0) {
            Some(link) => link.unwrap(),

            _ => {
                println!("error: no server jar available");
                exit(1);
            }
        }
            .to_owned();

        println!("info: server jar at '{}'", server_jar_url);

        let mut jar_file = match download(server_jar_url.as_str(), 3) {
            Some(x) => x,
            _ => exit(1)
        };

        match create_file(&install_repo.join(jar_version.as_str())) {
            Some(mut cache_file) => {
                match io::copy(&mut jar_file, &mut cache_file) {
                    Ok(_) => {
                        match fs::copy(&install_repo.join(jar_version.clone()), match create_file(&Path::new(".").join("server.jar")) {
                            Some(_) => {
                                let h = Path::new(".").join("server.jar");
                                h
                            }
                            None => {
                                println!("error: failed to create server.jar");
                                exit(1);
                            }
                        }) {
                            Ok(_) => {
                                println!("info: successfully cached jar in '{}' and installed", install_repo.display());
                            }
                            Err(e) => {
                                println!("error: successfully cached jar in '{}' but failed to install: {}", install_repo.display(), e);
                            }
                        }
                    }
                    Err(e) => {
                        match io::copy(&mut jar_file, &mut match create_file(&Path::new(".").join("server.jar")) {
                            Some(f) => f,
                            None => {
                                println!("error: failed to create server.jar");
                                exit(1);
                            }
                        }) {
                            Ok(_) => {
                                println!("warning: successfully installed but failed to cache jar: {}", e);
                            }
                            Err(e2) => {
                                println!("error: failed to cache ({}) and failed to install ({})", e, e2);
                            }
                        }
                    }
                }
            }
            None => {
                println!("warning: failed to cache file in repository '{}'", install_repo.display());
                match io::copy(&mut jar_file, &mut match create_file(&Path::new(".").join("server.jar")) {
                    Some(f) => f,
                    None => {
                        println!("error: failed to create server.jar");
                        exit(1);
                    }
                }) {
                    Ok(_) => {
                        println!("warning: successfully installed but failed to cache jar");
                    }
                    Err(e2) => {
                        println!("error: failed to cache and failed to install: {}", e2);
                    }
                }
            }
        }
    } else {
        let jar_file = jar_file.unwrap();
        println!("info: found cached jar at: {}", jar_file.display());

        match fs::copy(&jar_file, match create_file(&Path::new(".").join("server.jar")) {
            Some(_) => {
                let h = Path::new(".").join("server.jar");
                h
            }
            None => {
                println!("error: failed to create server.jar");
                exit(1);
            }
        }) {
            Ok(_) => {
                println!("info: successfully installed version {}", jar_version);
            }
            Err(e) => {
                println!("error: failed to install: {}", e);
            }
        }
    }
}