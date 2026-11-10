use super::*;

/// The fields of the cosign bundle that podnet checks.
#[derive(Debug, Deserialize)]
struct Bundle {
    #[serde(rename = "verificationMaterial")]
    material: Material,
    #[serde(rename = "messageSignature")]
    signature: MessageSignature,
}

#[derive(Debug, Deserialize)]
struct Material {
    certificate: Option<RawBytes>,
    #[serde(rename = "x509CertificateChain")]
    chain: Option<Chain>,
    #[serde(rename = "tlogEntries", default)]
    tlog: Vec<TlogEntry>,
}

#[derive(Debug, Deserialize)]
struct Chain {
    certificates: Vec<RawBytes>,
}

#[derive(Debug, Deserialize)]
struct RawBytes {
    #[serde(rename = "rawBytes")]
    raw: String,
}

#[derive(Debug, Deserialize)]
struct TlogEntry {
    #[serde(rename = "integratedTime")]
    integrated_time: String,
}

#[derive(Debug, Deserialize)]
struct MessageSignature {
    #[serde(rename = "messageDigest")]
    digest: MessageDigest,
    signature: String,
}

#[derive(Debug, Deserialize)]
struct MessageDigest {
    algorithm: String,
    digest: String,
}

fn b64(text: &str) -> Result<Vec<u8>> {
    base64::engine::general_purpose::STANDARD
        .decode(text.trim())
        .context("bad base64")
}

/// Check the cosign bundle `bundle` over `sums`, for the tag `tag`. See the
/// checks 1 to 3 of the module.
pub fn verify_bundle(bundle: &[u8], sums: &[u8], tag: &str) -> Result<()> {
    verify_bundle_for(bundle, sums, &identity(tag))
}

/// [`verify_bundle`] for the certificate identity `want`.
pub fn verify_bundle_for(bundle: &[u8], sums: &[u8], want: &str) -> Result<()> {
    use ring::digest::{digest, SHA256};
    let bundle: Bundle = serde_json::from_slice(bundle).context("decoding the cosign bundle")?;
    let leaf = match (&bundle.material.certificate, &bundle.material.chain) {
        (Some(cert), _) => b64(&cert.raw)?,
        (None, Some(chain)) => b64(&chain.certificates.first().context("an empty chain")?.raw)?,
        (None, None) => bail!("the bundle has no certificate"),
    };
    let time = bundle
        .material
        .tlog
        .first()
        .context("the bundle has no log entry")?
        .integrated_time
        .parse::<u64>()
        .context("the time of the log entry")?;

    // 1. The chain to the Fulcio roots, at the time of the log entry.
    use rustls::pki_types::pem::PemObject;
    let mut pems = rustls::pki_types::CertificateDer::pem_slice_iter(FULCIO_PEM)
        .collect::<std::result::Result<Vec<_>, _>>()
        .context("the embedded Fulcio roots")?;
    let root = pems.pop().context("no Fulcio root")?;
    let anchor = webpki::anchor_from_trusted_cert(&root)
        .context("the Fulcio root")?
        .to_owned();
    let leaf_der = rustls::pki_types::CertificateDer::from(leaf.clone());
    let end =
        webpki::EndEntityCert::try_from(&leaf_der).context("the certificate of the bundle")?;
    const CODE_SIGNING: &[u8] = &[0x2b, 0x06, 0x01, 0x05, 0x05, 0x07, 0x03, 0x03];
    end.verify_for_usage(
        webpki::ALL_VERIFICATION_ALGS,
        &[anchor],
        &pems,
        rustls::pki_types::UnixTime::since_unix_epoch(Duration::from_secs(time)),
        webpki::KeyUsage::required(CODE_SIGNING),
        None,
        None,
    )
    .map_err(|e| anyhow::anyhow!("the certificate does not chain to Fulcio: {e:?}"))?;

    // 2. The identity and the issuer.
    let (_, cert) =
        x509_parser::parse_x509_certificate(&leaf).context("parsing the certificate")?;
    let want = want.to_string();
    let mut uris = Vec::new();
    let mut issuer = None;
    for ext in cert.extensions() {
        match ext.parsed_extension() {
            x509_parser::extensions::ParsedExtension::SubjectAlternativeName(san) => {
                for name in &san.general_names {
                    if let x509_parser::extensions::GeneralName::URI(uri) = name {
                        uris.push(uri.to_string());
                    }
                }
            }
            _ => {
                let oid = ext.oid.to_id_string();
                if oid == "1.3.6.1.4.1.57264.1.1" {
                    issuer = Some(String::from_utf8_lossy(ext.value).into_owned());
                } else if oid == "1.3.6.1.4.1.57264.1.8" && issuer.is_none() {
                    // A DER UTF8String: tag, length, text.
                    issuer = ext
                        .value
                        .get(2..)
                        .map(|v| String::from_utf8_lossy(v).into_owned());
                }
            }
        }
    }
    if !uris.contains(&want) {
        bail!("the certificate names {uris:?}, not {want}");
    }
    if issuer.as_deref() != Some(ISSUER) {
        bail!("the certificate has the issuer {issuer:?}, not {ISSUER}");
    }

    // 3. The digest and the signature over SHA256SUMS.
    if bundle.signature.digest.algorithm != "SHA2_256" {
        bail!(
            "the bundle digest is {}, not SHA2_256",
            bundle.signature.digest.algorithm
        );
    }
    if b64(&bundle.signature.digest.digest)? != digest(&SHA256, sums).as_ref() {
        bail!("the digest of the bundle is not the SHA-256 of SHA256SUMS");
    }
    let key = cert.public_key().subject_public_key.data.to_vec();
    let signature = b64(&bundle.signature.signature)?;
    ring::signature::UnparsedPublicKey::new(&ring::signature::ECDSA_P256_SHA256_ASN1, key)
        .verify(sums, &signature)
        .map_err(|_| anyhow::anyhow!("the signature over SHA256SUMS is not valid"))?;
    Ok(())
}

/// The SHA-256 of `name` in `sums` (the lines of `sha256sum`).
pub fn sum_of(sums: &[u8], name: &str) -> Result<Vec<u8>> {
    let text = std::str::from_utf8(sums).context("SHA256SUMS is not text")?;
    for line in text.lines() {
        if let Some((hex, file)) = line.split_once("  ") {
            if file.trim_start_matches('*') == name {
                return (0..hex.len())
                    .step_by(2)
                    .map(|i| u8::from_str_radix(hex.get(i..i + 2).unwrap_or("zz"), 16))
                    .collect::<std::result::Result<Vec<u8>, _>>()
                    .context("a bad hash in SHA256SUMS");
            }
        }
    }
    bail!("SHA256SUMS has no line for {name}")
}
