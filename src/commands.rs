use crate::models::ToolRequest;
use anyhow::{Context, Result, bail};
use serde_json::Value;
use std::collections::BTreeMap;

fn text(values: &BTreeMap<String, Value>, key: &str, default: &str) -> Result<String> {
    match values.get(key) {
        None => Ok(default.to_owned()),
        Some(Value::String(value)) => Ok(value.clone()),
        Some(_) => bail!("{key} must be a string"),
    }
}

fn required(values: &BTreeMap<String, Value>, key: &str) -> Result<String> {
    let value = text(values, key, "")?;
    if value.trim().is_empty() {
        bail!("{key} is required");
    }
    Ok(value)
}

fn extra(argv: &mut Vec<String>, value: String) -> Result<()> {
    if !value.is_empty() {
        argv.extend(shell_words::split(&value).context("invalid additional_args quoting")?);
    }
    Ok(())
}

/// Build scanner argv without invoking a shell. This preserves the old API shape
/// while removing its command-injection boundary.
pub fn tool_command(tool: &str, request: &ToolRequest) -> Result<Vec<String>> {
    let v = &request.values;
    let mut argv = match tool {
        "nmap" => {
            let target = required(v, "target")?;
            let mut a = vec!["nmap".into()];
            a.extend(shell_words::split(&text(v, "scan_type", "-sCV")?)?);
            let ports = text(v, "ports", "")?;
            if !ports.is_empty() {
                a.extend(["-p".into(), ports]);
            }
            extra(&mut a, text(v, "additional_args", "-T4 -Pn")?)?;
            a.push(target);
            a
        }
        "gobuster" => {
            let mode = text(v, "mode", "dir")?;
            if !["dir", "dns", "fuzz", "vhost"].contains(&mode.as_str()) {
                bail!("mode must be one of dir, dns, fuzz, vhost");
            }
            let mut a = vec![
                "gobuster".into(),
                mode,
                "-u".into(),
                required(v, "url")?,
                "-w".into(),
                text(v, "wordlist", "/usr/share/wordlists/dirb/common.txt")?,
            ];
            extra(&mut a, text(v, "additional_args", "")?)?;
            a
        }
        "dirb" => {
            let mut a = vec![
                "dirb".into(),
                required(v, "url")?,
                text(v, "wordlist", "/usr/share/wordlists/dirb/common.txt")?,
            ];
            extra(&mut a, text(v, "additional_args", "")?)?;
            a
        }
        "nikto" => {
            let mut a = vec!["nikto".into(), "-h".into(), required(v, "target")?];
            extra(&mut a, text(v, "additional_args", "")?)?;
            a
        }
        "sqlmap" => {
            let mut a = vec![
                "sqlmap".into(),
                "-u".into(),
                required(v, "url")?,
                "--batch".into(),
            ];
            let data = text(v, "data", "")?;
            if !data.is_empty() {
                a.extend(["--data".into(), data]);
            }
            extra(&mut a, text(v, "additional_args", "")?)?;
            a
        }
        "hydra" => {
            let mut a = vec!["hydra".into(), "-t".into(), "4".into()];
            let username = text(v, "username", "")?;
            let username_file = text(v, "username_file", "")?;
            let password = text(v, "password", "")?;
            let password_file = text(v, "password_file", "")?;
            if !username.is_empty() {
                a.extend(["-l".into(), username]);
            } else if !username_file.is_empty() {
                a.extend(["-L".into(), username_file]);
            } else {
                bail!("username or username_file is required");
            }
            if !password.is_empty() {
                a.extend(["-p".into(), password]);
            } else if !password_file.is_empty() {
                a.extend(["-P".into(), password_file]);
            } else {
                bail!("password or password_file is required");
            }
            a.extend([required(v, "target")?, required(v, "service")?]);
            extra(&mut a, text(v, "additional_args", "")?)?;
            a
        }
        "john" => {
            let mut a = vec!["john".into()];
            let format = text(v, "format", "")?;
            if !format.is_empty() {
                a.push(format!("--format={format}"));
            }
            let wordlist = text(v, "wordlist", "/usr/share/wordlists/rockyou.txt")?;
            if !wordlist.is_empty() {
                a.push(format!("--wordlist={wordlist}"));
            }
            extra(&mut a, text(v, "additional_args", "")?)?;
            a.push(required(v, "hash_file")?);
            a
        }
        "wpscan" => {
            let mut a = vec!["wpscan".into(), "--url".into(), required(v, "url")?];
            extra(&mut a, text(v, "additional_args", "")?)?;
            a
        }
        "enum4linux" => {
            let mut a = vec!["enum4linux".into()];
            extra(&mut a, text(v, "additional_args", "-a")?)?;
            a.push(required(v, "target")?);
            a
        }
        "metasploit" => {
            let module = required(v, "module")?;
            if !module
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "/_-".contains(c))
            {
                bail!("module contains invalid characters");
            }
            let options = match v.get("options") {
                None => BTreeMap::new(),
                Some(Value::Object(values)) => {
                    values.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
                }
                Some(_) => bail!("options must be an object"),
            };
            let mut script = format!("use {module}; ");
            for (key, value) in options {
                if !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                    bail!("invalid option key: {key}");
                }
                let value = value
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| value.to_string());
                if value.contains(['\r', '\n', ';']) {
                    bail!("option {key} contains a command separator");
                }
                script.push_str(&format!("set {key} {value}; "));
            }
            script.push_str("exploit; exit");
            vec!["msfconsole".into(), "-q".into(), "-x".into(), script]
        }
        _ => bail!("unknown tool: {tool}"),
    };
    if argv.is_empty() || argv[0].is_empty() {
        bail!("empty command");
    }
    Ok(std::mem::take(&mut argv))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nmap_target_is_one_argument() {
        let request = ToolRequest {
            values: BTreeMap::from([
                (
                    "target".into(),
                    Value::String("example.test; touch /tmp/nope".into()),
                ),
                ("additional_args".into(), Value::String(String::new())),
            ]),
        };
        let argv = tool_command("nmap", &request).unwrap();
        assert_eq!(argv.last().unwrap(), "example.test; touch /tmp/nope");
    }
}
