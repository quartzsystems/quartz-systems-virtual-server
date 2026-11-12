// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The state churn of the power-pull test. The churn writes
//! `host.toml` and `network.toml` over and over through the state
//! store, each time a generation `n` whose every value follows from
//! `n`. Whatever moment the power goes, each file must afterwards be a
//! whole generation, or the whole file there was before the churn
//! began: [`check_host`] and [`check_network`] say which, or what is
//! wrong.

use std::net::{IpAddr, Ipv4Addr};

use state::{Host, Network};

/// What a churned host name starts with.
const NAME: &str = "churn-";

/// How many time servers generation `n` has.
fn servers(n: u64) -> usize {
    20 + usize::try_from(n % 20).unwrap_or(0)
}

/// How many name servers generation `n` has.
fn resolvers(n: u64) -> usize {
    5 + usize::try_from(n % 5).unwrap_or(0)
}

fn server(n: u64) -> String {
    format!("g{n}.invalid")
}

fn resolver(n: u64) -> IpAddr {
    let [.., a, b] = n.to_be_bytes();
    IpAddr::V4(Ipv4Addr::new(10, a, b, 1))
}

/// Generation `n` of `host.toml`, made from what the file held: the
/// serial number is the host's and stays.
pub fn host(before: &Host, n: u64) -> Host {
    Host {
        name: Some(format!("{NAME}{n}")),
        ntp_servers: vec![server(n); servers(n)],
        ..before.clone()
    }
}

/// Generation `n` of `network.toml`.
pub fn network(before: &Network, n: u64) -> Network {
    Network {
        dns: vec![resolver(n); resolvers(n)],
        ..before.clone()
    }
}

/// The generation `host` is, `None` when the churn never wrote it; an
/// error when it is no whole generation.
pub fn check_host(host: &Host) -> Result<Option<u64>, String> {
    let Some(n) = host
        .name
        .as_deref()
        .and_then(|name| name.strip_prefix(NAME))
    else {
        return if host.ntp_servers.is_empty() {
            Ok(None)
        } else {
            Err("time servers without a churned name".into())
        };
    };
    let n: u64 = n.parse().map_err(|_| format!("generation `{n}`"))?;
    let whole =
        host.ntp_servers.len() == servers(n) && host.ntp_servers.iter().all(|s| *s == server(n));
    if whole {
        Ok(Some(n))
    } else {
        Err(format!(
            "host.toml is not the whole of generation {n}: {} time servers",
            host.ntp_servers.len()
        ))
    }
}

/// The same for `network.toml`. Its generation is known by its first
/// name server alone, so only that the file is whole is checked.
pub fn check_network(network: &Network) -> Result<bool, String> {
    let Some(first) = network.dns.first() else {
        return Ok(false);
    };
    let same = network.dns.iter().all(|d| d == first);
    let count = network.dns.len();
    if same && (5..10).contains(&count) {
        Ok(true)
    } else {
        Err(format!(
            "network.toml is not a whole generation: {count} name servers"
        ))
    }
}

/// Churn the state files in `store` for ever, as fast as they can be
/// written.
pub fn run(store: &state::Store) -> ! {
    let host_before: Host = store.load().ok().flatten().unwrap_or_default();
    let network_before: Network = store.load().ok().flatten().unwrap_or_default();
    // Go on from the generation the last IPL's churn reached.
    let mut n = check_host(&host_before).ok().flatten().map_or(0, |n| n + 1);
    loop {
        let written = store
            .save(&host(&host_before, n))
            .and_then(|()| store.save(&network(&network_before, n)));
        if let Err(e) = written {
            eprintln!("churn: {e}");
            std::thread::sleep(std::time::Duration::from_secs(1));
        }
        n += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use state::store::{parse, render};

    #[test]
    fn a_generation_checks_and_survives_the_file() {
        let before = Host {
            serial: Some("M525400123456".into()),
            ..Host::default()
        };
        assert_eq!(check_host(&before), Ok(None));
        for n in [0, 1, 19, 20, 12_345] {
            let h = host(&before, n);
            assert_eq!(h.serial, before.serial);
            let back: Host = parse(&render(&h).unwrap()).unwrap();
            assert_eq!(check_host(&back), Ok(Some(n)));
            let net: Network = parse(&render(&network(&Network::default(), n)).unwrap()).unwrap();
            assert_eq!(check_network(&net), Ok(true));
        }
        assert_eq!(check_network(&Network::default()), Ok(false));
    }

    #[test]
    fn a_mixed_or_short_file_does_not_check() {
        let mut h = host(&Host::default(), 7);
        h.ntp_servers.pop();
        assert!(check_host(&h).is_err());
        let mut h = host(&Host::default(), 7);
        h.ntp_servers[3] = server(8);
        assert!(check_host(&h).is_err());
        let mut h = host(&Host::default(), 7);
        h.name = None;
        assert!(check_host(&h).is_err());
        let mut net = network(&Network::default(), 7);
        net.dns[1] = resolver(8);
        assert!(check_network(&net).is_err());
    }
}
