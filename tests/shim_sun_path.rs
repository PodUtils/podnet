//! The abstract name with the longest prefix and a full IPv6 literal: the
//! name is the 107 bytes of `sun_path` less its leading NUL, and the C
//! builder of the shim does not truncate it.

mod shim_harness;

use std::process::Command;

use podnet::vnet::shim;
use shim_harness::{build_program, have_compiler, run_pair};

/// H-05: with the longest prefix and a full IPv6 literal, the name is the
/// 107 bytes of `sun_path` less its leading NUL, and the C builder does not
/// truncate it.
#[test]
fn a_long_prefix_and_a_full_ipv6_name_fit_sun_path() {
    if !have_compiler() {
        eprintln!("skipping: no C compiler");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let shim = shim::build(dir.path()).unwrap();
    let prefix = "p".repeat(podnet::vnet::addr::MAX_PREFIX);
    let addr: std::net::Ipv6Addr = "ffff:ffff:ffff:ffff:ffff:ffff:ffff:ffff".parse().unwrap();
    let expected = podnet::vnet::VirtAddr::v6(addr, 65535)
        .abstract_name_with(&prefix)
        .unwrap();
    assert_eq!(expected.len(), podnet::vnet::addr::MAX_NAME);

    let server_src = r#"
#include <stdio.h>
#include <string.h>
#include <arpa/inet.h>
#include <unistd.h>
int main(void){
  int s=socket(AF_INET6,SOCK_STREAM,0);
  struct sockaddr_in6 a; memset(&a,0,sizeof a);
  a.sin6_family=AF_INET6; a.sin6_port=htons(65535);
  inet_pton(AF_INET6,"ffff:ffff:ffff:ffff:ffff:ffff:ffff:ffff",&a.sin6_addr);
  if(bind(s,(struct sockaddr*)&a,sizeof a)<0){perror("bind");return 1;}
  listen(s,4);
  printf("ready\n"); fflush(stdout);
  int c=accept(s,0,0);
  char b[8]={0}; int n=read(c,b,7);
  printf("server: read %d\n", n);
  write(c,"ok",2);
  close(c); close(s); return 0;
}
"#;
    let client_src = r#"
#include <stdio.h>
#include <string.h>
#include <arpa/inet.h>
#include <unistd.h>
int main(void){
  int s=socket(AF_INET6,SOCK_STREAM,0);
  struct sockaddr_in6 a; memset(&a,0,sizeof a);
  a.sin6_family=AF_INET6; a.sin6_port=htons(65535);
  inet_pton(AF_INET6,"ffff:ffff:ffff:ffff:ffff:ffff:ffff:ffff",&a.sin6_addr);
  if(connect(s,(struct sockaddr*)&a,sizeof a)<0){perror("connect");return 1;}
  write(s,"hi",2);
  char b[8]={0}; int n=read(s,b,7);
  printf("client: read %d\n", n);
  close(s); return 0;
}
"#;
    let server = build_program(dir.path(), "long-server", server_src);
    let client = build_program(dir.path(), "long-client", client_src);
    let mut server_cmd = Command::new(&server);
    server_cmd
        .env("LD_PRELOAD", &shim.path)
        .env("PODNET_VNET_LOG", "1")
        .env("PODNET_VNET_PREFIX", &prefix);
    let mut client_cmd = Command::new(&client);
    client_cmd
        .env("LD_PRELOAD", &shim.path)
        .env("PODNET_VNET_LOG", "1")
        .env("PODNET_VNET_PREFIX", &prefix);
    let (server_output, output) = run_pair(&mut server_cmd, &mut client_cmd);
    let server_stderr = String::from_utf8_lossy(&server_output.stderr);
    assert!(
        server_output.status.success(),
        "server failed: {server_stderr}"
    );
    assert!(
        output.status.success(),
        "client failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let client_stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        server_stderr.contains(&format!("bind -> {expected}")),
        "the bind line does not carry the full name: {server_stderr}"
    );
    assert!(
        client_stderr.contains(&format!("connect -> {expected}")),
        "the connect line does not carry the full name: {client_stderr}"
    );
    assert!(
        String::from_utf8_lossy(&server_output.stdout).contains("server: read 2"),
        "{}",
        String::from_utf8_lossy(&server_output.stdout)
    );

    // A prefix that cannot build a name is ignored with a line, not cut.
    let bad = "p".repeat(podnet::vnet::addr::MAX_PREFIX + 1);
    let mut bad_server = Command::new(&server);
    bad_server
        .env("LD_PRELOAD", &shim.path)
        .env("PODNET_VNET_LOG", "1")
        .env("PODNET_VNET_PREFIX", &bad);
    let mut bad_client = Command::new(&client);
    bad_client
        .env("LD_PRELOAD", &shim.path)
        .env("PODNET_VNET_LOG", "1")
        .env("PODNET_VNET_PREFIX", &bad);
    let (bad_server_output, bad_output) = run_pair(&mut bad_server, &mut bad_client);
    let bad_stderr = String::from_utf8_lossy(&bad_server_output.stderr);
    assert!(
        bad_stderr.contains("is not 1 to 59 bytes"),
        "no line about the ignored prefix: {bad_stderr}"
    );
    let default = podnet::vnet::VirtAddr::v6(addr, 65535)
        .abstract_name_with(podnet::vnet::addr::DEFAULT_PREFIX)
        .unwrap();
    assert!(
        bad_stderr.contains(&format!("bind -> {default}")),
        "the fallback name is not the default one: {bad_stderr}"
    );
    assert!(bad_output.status.success());

    // Even without PODNET_VNET_LOG, the ignored prefix prints to stderr.
    let mut quiet_server = Command::new(&server);
    quiet_server
        .env("LD_PRELOAD", &shim.path)
        .env_remove("PODNET_VNET_LOG")
        .env("PODNET_VNET_PREFIX", &bad);
    let mut quiet_client = Command::new(&client);
    quiet_client
        .env("LD_PRELOAD", &shim.path)
        .env_remove("PODNET_VNET_LOG")
        .env("PODNET_VNET_PREFIX", &bad);
    let (quiet_server_output, quiet_client_output) = run_pair(&mut quiet_server, &mut quiet_client);
    let quiet_stderr = String::from_utf8_lossy(&quiet_server_output.stderr);
    assert!(
        quiet_stderr.contains("is not 1 to 59 bytes"),
        "no line about the ignored prefix without PODNET_VNET_LOG: {quiet_stderr}"
    );
    assert!(quiet_client_output.status.success());
}
