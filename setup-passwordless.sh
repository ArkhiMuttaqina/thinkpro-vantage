#!/bin/bash
set -e

echo "Setting up passwordless access for ThinkPro Vantage..."
echo ""

# Get the current user
CURRENT_USER=$(whoami)

# Check if user is in sudo or wheel group
if groups $CURRENT_USER | grep -qE '\b(sudo|wheel)\b'; then
    echo "✓ User '$CURRENT_USER' is in sudo/wheel group"
else
    echo "✗ User '$CURRENT_USER' is NOT in sudo/wheel group"
    echo "  You need to be in the sudo or wheel group for passwordless access"
    exit 1
fi

REPO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Install the committed helper rather than a copy of it.
#
# This script used to carry its own inline copy, and the copy had drifted: it
# predated the watchdog arm and the fixed reads, so a machine set up with this
# script had a helper whose whitelist was older than the app talking to it.
# Anything the app cannot do through that helper it falls back to running bash
# under pkexec for, which is the prompt this script exists to remove.
echo "Installing the privileged helper..."
sudo install -Dm0755 "$REPO_DIR/packaging/helper/thinkpro-ctl" /usr/local/bin/thinkpro-ctl
echo "✓ Helper installed at /usr/local/bin/thinkpro-ctl"

# Create polkit rules directory if it doesn't exist
sudo mkdir -p /etc/polkit-1/rules.d

# Create the rule file (only allows the dedicated helper, not arbitrary bash)
#
# Also the committed rule, and that one matters more than it looks: the inline
# copy granted any wheel or sudo session, so a remote SSH session inherited
# passwordless root-adjacent exec. The generated rule requires the session to be
# local and active first.
echo "Creating polkit rule..."
sudo install -Dm0644 "$REPO_DIR/packaging/polkit/50-thinkpro-vantage.rules" \
    /etc/polkit-1/rules.d/50-thinkpro-vantage.rules

echo "✓ Polkit rule created at /etc/polkit-1/rules.d/50-thinkpro-vantage.rules"
echo ""
echo "Reloading polkit..."
sudo systemctl reload polkit 2>/dev/null || sudo killall -HUP polkitd 2>/dev/null || true

echo ""
echo "✓ Setup complete!"
echo ""
echo "ThinkPro Vantage fan control will now work without asking for a password."
echo "Restart the app to test."
