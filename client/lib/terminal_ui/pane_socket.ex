defmodule TerminalUi.PaneSocket do
  use WebSockex

  def start_link(pane_id) do
    url = "ws://localhost:3000/panes/#{pane_id}/stream"
    name = {:via, Registry, {TerminalUi.PaneRegistry, pane_id}}

    WebSockex.start_link(url, __MODULE__, %{pane_id: pane_id},
      name: name,
      handle_initial_conn_failure: true,
      async: false
    )
  end

  def handle_frame({:text, _json}, state) do
    case TerminalUi.TerminalClient.get_screen(state.pane_id) do
      {:ok, screen} ->
        Phoenix.PubSub.broadcast(
          TerminalUi.PubSub,
          "pane:#{state.pane_id}",
          {:screen_update, state.pane_id, screen}
        )

      _ ->
        :ok
    end

    {:ok, state}
  end

  def handle_frame({:binary, _}, state), do: {:ok, state}

  def handle_ping(_, state), do: {:reply, {:pong, ""}, state}

  def handle_disconnect(_, state), do: {:reconnect, state}
end
