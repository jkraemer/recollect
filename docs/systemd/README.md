# Systemd User Service Setup

Run Recollect as a systemd user service for automatic startup and management.

## Installation

1. Copy the sample start script and edit for your Ruby version manager:

```bash
cp docs/systemd/start-service.sample bin/start-service
chmod +x bin/start-service
# Edit bin/start-service to uncomment your Ruby version manager
```

2. Copy the systemd unit file and edit the path:

```bash
mkdir -p ~/.config/systemd/user
cp docs/systemd/recollect.service ~/.config/systemd/user/
# Edit ~/.config/systemd/user/recollect.service to set correct ExecStart path
```

3. Enable and start the service:

```bash
systemctl --user daemon-reload
systemctl --user enable recollect
systemctl --user start recollect
```

## Management Commands

```bash
systemctl --user status recollect   # Check service status
systemctl --user restart recollect  # Restart service
systemctl --user stop recollect     # Stop service
journalctl --user -u recollect -f   # View logs
```

## Headless Operation

To run the service without an active login session:

```bash
loginctl enable-linger $USER
```

## Environment Variables

Configure in the systemd unit file using `Environment=` directives:

| Variable | Default | Description |
|----------|---------|-------------|
| `RECOLLECT_DATA_DIR` | `~/.recollect` | Data storage directory |
| `RECOLLECT_HOST` | `127.0.0.1` | Server bind address |
| `RECOLLECT_PORT` | `7326` | Server port |
| `WEB_CONCURRENCY` | `2` | Puma worker processes |
| `PUMA_MAX_THREADS` | `5` | Threads per worker |
| `ENABLE_VECTORS` | (not set) | Enable semantic vector search |

Example:

```ini
[Service]
Environment=RECOLLECT_PORT=9000
Environment=WEB_CONCURRENCY=4
ExecStart=/path/to/recollect/bin/start-service
```
