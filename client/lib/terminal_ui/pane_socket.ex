defmodule TerminalUi.PaneSocket do
  use WebSockex

  require Logger

  @heartbeat_interval_ms 5_000
  @heartbeat_timeout_ms 5_000
  @heartbeat_payload "pane-heartbeat"

  def start_link(pane_id) do
    conn = TerminalUi.TerminalClient.ws_conn("/panes/#{pane_id}/stream")
    name = {:via, Registry, {TerminalUi.PaneRegistry, pane_id}}

    WebSockex.start_link(conn, __MODULE__, new_state(pane_id),
      name: name,
      handle_initial_conn_failure: true,
      # Don't block the caller while the pane stream connects.
      async: true
    )
  end

  def handle_connect(_conn, state) do
    {:ok, begin_heartbeat(state)}
  end

  def handle_frame({:text, _json}, state) do
    state = note_socket_activity(state)

    case TerminalUi.TerminalClient.get_screen(state.pane_id) do
      {:ok, screen} ->
        Phoenix.PubSub.broadcast(
          TerminalUi.PubSub,
          "pane:#{state.pane_id}",
          {:screen_update, state.pane_id, screen}
        )

      {:error, :not_found} ->
        :ok

      {:error, reason} ->
        Logger.warning(
          "failed to fetch screen for pane #{state.pane_id} after stream frame: #{inspect(reason)}"
        )

      _ ->
        :ok
    end

    {:ok, state}
  end

  def handle_frame({:binary, _}, state), do: {:ok, note_socket_activity(state)}

  def handle_ping(:ping, state), do: {:reply, :pong, note_socket_activity(state)}

  def handle_ping({:ping, payload}, state) do
    {:reply, {:pong, payload}, note_socket_activity(state)}
  end

  def handle_pong(:pong, state), do: {:ok, note_socket_activity(state)}
  def handle_pong({:pong, _payload}, state), do: {:ok, note_socket_activity(state)}

  def handle_info({:heartbeat_tick, nonce}, state) do
    cond do
      state.connection_nonce != nonce ->
        {:ok, state}

      state.pending_heartbeat_token ->
        {:ok, schedule_heartbeat_tick(%{state | heartbeat_tick_ref: nil})}

      recently_active?(state) ->
        {:ok, schedule_heartbeat_tick(%{state | heartbeat_tick_ref: nil})}

      true ->
        token = make_ref()

        timeout_ref =
          Process.send_after(self(), {:heartbeat_timeout, nonce, token}, @heartbeat_timeout_ms)

        state =
          state
          |> cancel_timer(:heartbeat_timeout_ref)
          |> Map.put(:heartbeat_tick_ref, nil)
          |> Map.put(:heartbeat_timeout_ref, timeout_ref)
          |> Map.put(:pending_heartbeat_token, token)
          |> schedule_heartbeat_tick()

        {:reply, {:ping, @heartbeat_payload}, state}
    end
  end

  def handle_info({:heartbeat_timeout, nonce, token}, state) do
    cond do
      state.connection_nonce != nonce ->
        {:ok, state}

      state.pending_heartbeat_token != token ->
        {:ok, state}

      true ->
        Logger.warning("pane stream heartbeat timed out for #{state.pane_id}; reconnecting")

        state =
          state
          |> cancel_timer(:heartbeat_timeout_ref)
          |> Map.put(:pending_heartbeat_token, nil)

        {:close, {4000, "heartbeat_timeout"}, state}
    end
  end

  def handle_info(_message, state), do: {:ok, state}

  def handle_disconnect(conn_status, state) do
    state = end_heartbeat(state)

    case TerminalUi.TerminalClient.get_pane(state.pane_id) do
      {:ok, %{"terminated" => true}} ->
        {:ok, state}

      {:error, :not_found} ->
        {:ok, state}

      {:error, reason} ->
        Logger.warning(
          "pane stream disconnect for #{state.pane_id}; reconnecting after pane lookup error #{inspect(reason)}"
        )

        {:reconnect, state}

      {:ok, _pane} ->
        Logger.warning(
          "pane stream disconnect for #{state.pane_id}; reconnecting after #{inspect(conn_status.reason)}"
        )

        {:reconnect, state}
    end
  end

  defp new_state(pane_id) do
    %{
      pane_id: pane_id,
      connection_nonce: nil,
      heartbeat_tick_ref: nil,
      heartbeat_timeout_ref: nil,
      pending_heartbeat_token: nil,
      last_activity_ms: nil
    }
  end

  # Pane streams are naturally idle when the PTY is quiet, so heartbeat pings
  # are only sent after a period with no inbound frames.
  defp begin_heartbeat(state) do
    nonce = make_ref()

    state
    |> end_heartbeat()
    |> Map.put(:connection_nonce, nonce)
    |> Map.put(:last_activity_ms, now_ms())
    |> schedule_heartbeat_tick()
  end

  defp end_heartbeat(state) do
    state
    |> cancel_timer(:heartbeat_tick_ref)
    |> cancel_timer(:heartbeat_timeout_ref)
    |> Map.put(:heartbeat_tick_ref, nil)
    |> Map.put(:heartbeat_timeout_ref, nil)
    |> Map.put(:pending_heartbeat_token, nil)
  end

  defp note_socket_activity(state) do
    state
    |> cancel_timer(:heartbeat_timeout_ref)
    |> Map.put(:heartbeat_timeout_ref, nil)
    |> Map.put(:pending_heartbeat_token, nil)
    |> Map.put(:last_activity_ms, now_ms())
  end

  defp schedule_heartbeat_tick(state) do
    state = cancel_timer(state, :heartbeat_tick_ref)

    tick_ref =
      Process.send_after(
        self(),
        {:heartbeat_tick, state.connection_nonce},
        @heartbeat_interval_ms
      )

    %{state | heartbeat_tick_ref: tick_ref}
  end

  defp recently_active?(state) do
    case state.last_activity_ms do
      nil -> false
      last_activity_ms -> now_ms() - last_activity_ms < @heartbeat_interval_ms
    end
  end

  defp cancel_timer(state, key) do
    case Map.get(state, key) do
      nil ->
        state

      timer_ref ->
        Process.cancel_timer(timer_ref)
        Map.put(state, key, nil)
    end
  end

  defp now_ms, do: System.monotonic_time(:millisecond)
end
