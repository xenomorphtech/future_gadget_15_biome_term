defmodule TerminalUi.PaneSocket do
  use WebSockex

  def start_link(pane_id) do
    url = TerminalUi.TerminalClient.ws_url("/panes/#{pane_id}/stream")
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

  def handle_disconnect(_conn_status, state) do
    case TerminalUi.TerminalClient.get_pane(state.pane_id) do
      {:ok, %{"terminated" => true}} ->
        Phoenix.PubSub.broadcast(
          TerminalUi.PubSub,
          "panes",
          {:pane_terminated, state.pane_id}
        )

        {:ok, state}

      {:error, :not_found} ->
        Phoenix.PubSub.broadcast(
          TerminalUi.PubSub,
          "panes",
          {:pane_deleted, state.pane_id}
        )

        {:ok, state}

      _ ->
        {:reconnect, state}
    end
  end
end
