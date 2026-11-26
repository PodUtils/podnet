# shellcheck shell=sh
# docs/host-runtime.md: a policy file. The set-up writes rules.json, the
# policy file of the document.

setup() {
    cat > rules.json << 'EOF'
{"default": "refuse", "rules": [
  {"name": "api", "host": "*.example.net", "port": 443, "action": "direct"},
  {"name": "dns", "host": "1.1.1.1", "port": "53", "family": "ipv4", "action": "proxy"},
  {"name": "no-ads", "host": "ads.example.com", "action": "refuse"}
]}
EOF
}
