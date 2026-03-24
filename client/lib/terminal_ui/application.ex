defmodule TerminalUi.Application do
  # See https://hexdocs.pm/elixir/Application.html
  # for more information on OTP Applications
  @moduledoc false

  use Application

  @impl true
  def start(_type, _args) do
    lifecycle_children =
      if Application.get_env(:terminal_ui, :start_pane_lifecycle_socket, true) do
        [TerminalUi.PaneLifecycleSocket]
      else
        []
      end

    children =
      [
        {Phoenix.PubSub, name: TerminalUi.PubSub},
        lifecycle_children,
        {Registry, keys: :unique, name: TerminalUi.PaneRegistry},
        TerminalUi.PaneSupervisor,
        TerminalUiWeb.Telemetry,
        {DNSCluster, query: Application.get_env(:terminal_ui, :dns_cluster_query) || :ignore},
        # Start to serve requests, typically the last entry
        TerminalUiWeb.Endpoint
      ]
      |> List.flatten()

    # See https://hexdocs.pm/elixir/Supervisor.html
    # for other strategies and supported options
    opts = [strategy: :one_for_one, name: TerminalUi.Supervisor]
    Supervisor.start_link(children, opts)
  end

  # Tell Phoenix to update the endpoint configuration
  # whenever the application is updated.
  @impl true
  def config_change(changed, _new, removed) do
    TerminalUiWeb.Endpoint.config_change(changed, removed)
    :ok
  end
end
