defmodule DashboardWeb do
  def live_view do
    quote do
      use Phoenix.LiveView,
        layout: {DashboardWeb.Layouts, :root}

      unquote(html_helpers())
    end
  end

  def html_helpers do
    quote do
      import Phoenix.HTML
      import Phoenix.Component
    end
  end
end
