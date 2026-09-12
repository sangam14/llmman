import Config

config :dashboard,
  namespace: Dashboard

# Configures the endpoint
config :dashboard, DashboardWeb.Endpoint,
  url: [host: "localhost"],
  adapter: Bandit.PhoenixAdapter,
  render_errors: [
    formats: [html: DashboardWeb.ErrorHTML, json: DashboardWeb.ErrorJSON],
    layout: false
  ],
  pubsub_server: Dashboard.PubSub,
  live_view: [signing_salt: "llmman_super_secret_signing_salt_2026"]

# Configure Jason for JSON parsing in Phoenix
config :phoenix, :json_library, Jason

import_config "#{config_env()}.exs"
