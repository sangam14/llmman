import Config

config :dashboard, DashboardWeb.Endpoint,
  http: [ip: {127, 0, 0, 1}, port: 4040],
  check_origin: false,
  code_reloader: false,
  debug_errors: true,
  secret_key_base: "B+9xN/j3gQWkK4M8y5D2jF7uZ0vA6eL1cT9oR3pI5sH7mV2bE8wN6xY1qP4uJ7zO"

config :logger, :console, format: "[$level] $message\n"

config :phoenix, :stacktrace_depth, 20
config :phoenix, :plug_init_mode, :runtime
