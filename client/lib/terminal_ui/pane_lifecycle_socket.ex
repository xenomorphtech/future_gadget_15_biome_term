defmodule TerminalUi.PaneLifecycleSocket do
  use WebSockex

  def start_link(_opts) do
    WebSockex.start_link(
      TerminalUi.TerminalClient.ws_url("/panes/lifecycle"),
      __MODULE__,
      %{},
      name: __MODULE__,
      handle_initial_conn_failure: true,
      async: true
    )
  end

  def handle_frame({:text, json}, state) do
    case Jason.decode(json) do
      {:ok, %{"type" => "snapshot", "panes" => panes}} ->
        broadcast({:panes_snapshot, panes})

      {:ok, %{"type" => "created", "pane" => pane}} ->
        broadcast({:pane_created, pane})

      {:ok, %{"type" => "deleted", "id" => id}} ->
        broadcast({:pane_deleted, id})

      _ ->
        :ok
    end

    {:ok, state}
  end

  def handle_frame({:binary, _}, state), do: {:ok, state}

  def handle_ping(_, state), do: {:reply, {:pong, ""}, state}

  def handle_disconnect(_conn_status, state), do: {:reconnect, state}

  defp broadcast(message) do
    Phoenix.PubSub.broadcast(TerminalUi.PubSub, "panes", message)
  end
end
