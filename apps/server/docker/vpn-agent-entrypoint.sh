#!/bin/sh
set -eu

# This script is only for a dedicated container network namespace, never host
# networking. The lock prevents concurrent agents from using the socket volume.
if [ "${1:-}" != "--locked" ]; then
    exec flock --exclusive --nonblock --no-fork \
        /run/apprelay/agent.lock "$0" --locked
fi

# Recover from SIGKILL/container restarts after acquiring exclusive ownership.
# Both paths are reserved for this deployment, not administrator-managed VPNs.
rm -f /run/apprelay/vpn-agent.sock
if ip link show dev wg-apprelay >/dev/null 2>&1; then
    ip link delete dev wg-apprelay
fi

# Keep the iptables lock on the writable runtime volume, not the read-only root.
export XTABLES_LOCKFILE=/run/apprelay/xtables.lock

vpn_address="${APPRELAY_VPN_ADDRESS:-10.77.0.1/24}"
vpn_address="${vpn_address%/*}"

# Dynamic pairing listeners use the namespace's explicitly configured ephemeral
# TCP range. The HTTP API is reached through the HTTPS tunnel, not WireGuard.
# Nothing on the VPN may reach other addresses, Docker services, another peer,
# or the internet through forwarding.
iptables -w -P FORWARD DROP
iptables -w -N APPRELAY_VPN_INPUT 2>/dev/null \
    || iptables -w -F APPRELAY_VPN_INPUT
iptables -w -A APPRELAY_VPN_INPUT -d "$vpn_address" \
    -m conntrack --ctstate ESTABLISHED,RELATED -j ACCEPT
iptables -w -A APPRELAY_VPN_INPUT -d "$vpn_address" \
    -p tcp --dport 32768:60999 -j ACCEPT
iptables -w -A APPRELAY_VPN_INPUT -d "$vpn_address" \
    -p udp --dport 5354 -j ACCEPT
iptables -w -A APPRELAY_VPN_INPUT -j DROP
if ! iptables -w -C INPUT -i wg-apprelay -j APPRELAY_VPN_INPUT 2>/dev/null; then
    iptables -w -I INPUT 1 -i wg-apprelay -j APPRELAY_VPN_INPUT
fi

exec vpn-agent
