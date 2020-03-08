extern crate app_dirs;
extern crate clap;
extern crate regex;
extern crate reqwest;
extern crate select;

use std::collections::HashSet;
use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::exit;

use clap::{App, Arg, value_t};
use regex::Regex;
use reqwest::blocking::{get, Response};
use select::document::Document;
use select::predicate::Name;

const APP_INFO: app_dirs::AppInfo = app_dirs::AppInfo { name: "mc_update", author: "u296" };
const DOWNLOAD_BATCH_SIZE: usize = 5;


fn create_file(path: &Path) -> Result<fs::File, io::Error> {
    match fs::File::create(&path) {
        Ok(f) => Ok(f),
        Err(e) => {
            println!("warning: failed to create file '{}': {}", path.display(), e);
            Err(e)
        }
    }
}


trait FileWriter {
    fn write(&mut self, target: &Path) -> Result<(), io::Error>;
}

impl FileWriter for Response {
    fn write(&mut self, target: &Path) -> Result<(), io::Error> {
        match io::copy(self, &mut create_file(target)?) {
            Ok(_) => Ok(()),
            Err(e) => Err(e)
        }
    }
}

impl FileWriter for PathBuf {
    fn write(&mut self, target: &Path) -> Result<(), io::Error> {
        match fs::copy(self, target) {
            Ok(_) => Ok(()),
            Err(e) => Err(e)
        }
    }
}


fn prompt_continue(msg: &str) {
    println!("warning: {}\ncontinue? [Y/n]", msg);

    let mut input = String::new();
    io::stdin().read_line(&mut input).expect("error: failed to read stdin");
    input.to_ascii_lowercase();
    if input.contains("n") {
        exit(0);
    }
}

fn download(url: &str, max_tries: &Option<usize>) -> Option<Response> {
    println!("info: beginning download for url: {}", url);
    let mut attempt: usize = 1;
    let upper_bound: String = match max_tries {
        Some(x) => x.to_string(),
        None => "∞".to_string()
    };

    'out: loop {
        for _ in 0..DOWNLOAD_BATCH_SIZE {
            match max_tries {
                Some(x) => {
                    if attempt > *x {
                        return None;
                    }
                },
                _ => {}
            }
            match get(url) {
                Ok(response) => {
                    println!("info: ({}/{}) get request succeeded", attempt, upper_bound);
                    break 'out Some(response);
                }
                Err(e) => {
                    if let Some(code) = e.status() {
                        println!("warning: ({}/{}) get request failed with code {}: {}", attempt, upper_bound, code.as_u16(), code.canonical_reason().unwrap_or("no canonical reason"));
                    } else {
                        println!("warning: ({}/{}) get request failed", attempt, upper_bound);
                    }
                }
            }
            attempt += 1;
        }
        prompt_continue("batch failed, continue to retry");
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
        .arg(Arg::with_name("max_download_attempts")
            .short("m")
            .long("max_download_attempts")
            .help("sets the max amount of times to try to download a file, defaults to infinite")
            .takes_value(true))
        .arg(Arg::with_name("JAR_VERSION")
            .help("the jar version to update to, e.g. '1.12.2'")
            .required(true)
            .index(1))
        .arg(Arg::with_name("INSTALL_DIRECTORY")
            .help("the directory to install the server.jar into, defaults to '.'")
            .default_value(".")
            .index(2))
        .get_matches();

    let jar_version = matches.value_of("JAR_VERSION").unwrap().to_string();

    let max_download_attempts = match value_t!(matches, "max_download_attempts", usize)
        .unwrap_or_else(|e| {
            prompt_continue(&format!("could not interpret argument 'max_download_attempts' as a number, setting infinite limit: {}", e));
            std::usize::MAX
        }) {
        std::usize::MAX => None,
        x => Some(x),
    };

    //TODO

    let r = Regex::new(r#"(\.\.)|/|\\"#).unwrap();

    if r.is_match(jar_version.as_str()) {
        println!("error: jar version contains invalid characters");
        exit(1);
    }

    let local_repo_path = PathBuf::from("./mc_update");
    let global_repo_path = app_dirs::get_app_root(app_dirs::AppDataType::UserCache, &APP_INFO)
        .expect("error: failed to create global cache dir");
    let install_path = PathBuf::from(matches.value_of("INSTALL_DIRECTORY").unwrap());

    let mut default_repos = HashSet::new();
    default_repos.insert(global_repo_path);
    default_repos.insert(install_path.as_path().join("mc_update"));
    default_repos.insert(local_repo_path);

    let existing_repos = {
        let mut existing_repos = HashSet::new();


        let user_repos: HashSet<PathBuf> = matches.values_of("repo")
            .unwrap_or_default()
            .chain(
                matches.values_of("install_repo")
                    .unwrap_or_default()
            )
            .map(|x| PathBuf::from(x))
            .collect();


        for repo in default_repos.iter() {
            if repo.exists() {
                if repo.is_dir() {
                    existing_repos.insert(repo.clone());
                } else {
                    prompt_continue(&format!("the default repo '{}' appears to not be a directory", repo.display()));
                }
            }
        }

        for repo in user_repos.iter() {
            if repo.exists() {
                if repo.is_dir() {
                    existing_repos.insert(repo.clone());
                    continue;
                }
                println!("warning: '{}' is not a directory, ignoring", repo.display());
            } else {
                println!("warning: the repo '{}' does not exist", repo.display());
            }
        };
        existing_repos
    };
    let mut jar_file_path = None;

    'out: for repo in existing_repos {
        for entry in repo.read_dir().expect(format!("error: failed to read directory: '{}'", repo.display()).as_str()) {
            if let Ok(entry) = entry {
                if entry.path().is_file() && entry.file_name().as_os_str() == jar_version.as_str() {
                    jar_file_path = Some(entry.path());
                    break 'out;
                }
            }
        }
    }
    match fs::create_dir_all(&install_path) {
        Ok(_) => (),
        Err(e) => match e.kind() {
            io::ErrorKind::AlreadyExists => (),
            _ => {
                prompt_continue(&format!("failed to create install directory: {}", e));
            }
        }
    };

    let mut jar_content;
    let mut cache_file_path_buffer;

    let target_writer: &mut dyn FileWriter = {
        if let Some(jar_file_path) = &mut jar_file_path {
            // jar file is on disk
            jar_file_path
        } else {
            // jar file is not on disk
            let mut install_repo = None;

            let user_install_repos = matches.values_of("install_repo")
                .unwrap_or_default()
                .map(|x| PathBuf::from(x))
                .collect::<HashSet<PathBuf>>();

            for repo in user_install_repos.union(&default_repos).into_iter() {
                if repo.exists() {
                    if repo.is_dir() {
                        install_repo = Some(repo);
                        break;
                    }
                    prompt_continue(&format!("potential install repo '{}' is not a directory", repo.display()));
                    continue;
                } else {
                    match fs::create_dir_all(repo) {
                        Ok(_) => {
                            println!("info: created repo at '{}'", repo.display());
                            install_repo = Some(repo);
                            break;
                        }
                        Err(e) => {
                            prompt_continue(&format!("failed to create installation repo at '{}': '{}'", repo.display(), e));
                        }
                    }
                }
            }

            let install_repo = install_repo.expect("error: failed to select or create an install repo");

            let target = format!("https://mcversions.net/download/{}", jar_version);

            let page = match download(target.as_str(), &max_download_attempts) {
                Some(x) => x.text().unwrap(),
                _ => exit(1),
            };


            let jar_url = match Document::from(page.as_str()).find(Name("a"))
                .filter_map(|n| {
                    if n.attr("href")?.contains("server.jar") {
                        Some(n.attr("href"))
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
                .get(0) {
                Some(link) => link.unwrap(),

                _ => {
                    println!("error: failed to locate server jar");
                    exit(1);
                }
            }.to_owned();

            println!("info: server jar at '{}'", jar_url);

            jar_content = match download(jar_url.as_str(), &max_download_attempts) {
                Some(x) => x,
                None => {
                    println!("error: ran out of attempts to download jar");
                    exit(1);
                }
            };

            let tmp_writer: &mut dyn FileWriter = &mut jar_content;

            cache_file_path_buffer = install_repo.join(jar_version.as_str());

            match tmp_writer.write(&cache_file_path_buffer) {
                Ok(_) => {
                    println!("info: successfully cached jar in '{}'", install_repo.display());
                    &mut cache_file_path_buffer
                }
                Err(e) => {
                    println!("warning: failed to cache jar: {}", e);
                    &mut jar_content
                }
            }
        }
    };

    match target_writer.write(&install_path.join("server.jar")) {
        Ok(_) => {
            println!("info: successfully installed server jar with version '{}' into '{}'", jar_version, install_path.display());
        },
        Err(e) => {
            println!("error: failed to install server jar into '{}': {}", install_path.display(), e);
            exit(1);
        }
    }
}