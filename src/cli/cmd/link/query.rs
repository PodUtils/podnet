//! The query of an address, split off before any socket opens (T-306).

use anyhow::Result;

use podnet::carriers;

use super::{names_of, CarrierArgs};

/// Split the query off `address` and check it before any socket opens
/// (T-306). The key `carrier` takes the place of --carrier. A key that has
/// no effect yet exits 64. The keys `pad` and `compress` need --seal (exit
/// 64), and they set `carrier.record`. The result is the address with no
/// query.
pub(super) fn apply_query(address: &str, carrier: &mut CarrierArgs) -> Result<String> {
    carrier.normalize();
    let (base, query) = podnet::link_query::split(address)?;
    if let Some(name) = &query.carrier {
        carrier.carrier.clone_from(name);
        carrier.spec_address = None;
    }
    let base = if carrier.up_carrier.is_some() {
        carrier.carrier = podnet::link_split::NAME.to_string();
        if !base.contains('|') {
            return Err(podnet::exit::fault(
                podnet::exit::Fault::Usage,
                anyhow::anyhow!("--up-carrier and --down-carrier: give the address UP|DOWN, the address of each half"),
            ));
        }
        base.to_string()
    } else {
        carriers::check_query(&carrier.carrier, &query)?;
        let base = carriers::unix_path(&carrier.carrier, base);
        carriers::check_address(&carrier.carrier, &base)?;
        base
    };
    query.require_wired()?;
    if !carrier.seal
        && (query.pad.is_some()
            || query.compress.is_some()
            || query.profile.is_some()
            || query.firstflight.is_some())
    {
        return Err(podnet::exit::fault(
            podnet::exit::Fault::Usage,
            anyhow::anyhow!(
                "the query keys pad, compress, profile and firstflight seal the records: give --seal"
            ),
        ));
    }
    carrier.record = match carrier.seal {
        true => Some(podnet::link_record::Options::of_query(&query)?),
        false => None,
    };
    carrier.tune = podnet::link_keyed::Tune::of_query(&carrier.carrier, &query)?;
    let names = names_of(carrier);
    if !names.iter().any(|n| n == "dns")
        && (carrier.dns_server.is_some() || carrier.dns_zone.is_some())
    {
        return Err(podnet::exit::fault(
            podnet::exit::Fault::Usage,
            anyhow::anyhow!("--dns-server and --dns-zone are for the dns carrier"),
        ));
    }
    carrier.tune.dns_server = carrier.dns_server;
    carrier.tune.dns_zone.clone_from(&carrier.dns_zone);
    let s3 = carrier.s3_endpoint.is_some()
        || carrier.s3_region.is_some()
        || carrier.s3_credentials.is_some()
        || carrier.s3_prune;
    if !names.iter().any(|n| n == "s3") && s3 {
        return Err(podnet::exit::fault(
            podnet::exit::Fault::Usage,
            anyhow::anyhow!("--s3-endpoint, --s3-region, --s3-credentials and --s3-prune are for the s3 carrier"),
        ));
    }
    carrier.tune.s3_endpoint.clone_from(&carrier.s3_endpoint);
    carrier.tune.s3_region.clone_from(&carrier.s3_region);
    carrier
        .tune
        .s3_credentials
        .clone_from(&carrier.s3_credentials);
    carrier.tune.s3_prune = carrier.s3_prune;
    Ok(base)
}
