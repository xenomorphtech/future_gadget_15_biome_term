defmodule TerminalUi.PaneSocketTest do
  use ExUnit.Case, async: true

  test "sends a heartbeat ping after the stream has been idle" do
    {:ok, state} = TerminalUi.PaneSocket.handle_connect(nil, %{pane_id: "pane-1"})

    stale_state = %{state | last_activity_ms: System.monotonic_time(:millisecond) - 10_000}

    {:reply, {:ping, "pane-heartbeat"}, state} =
      TerminalUi.PaneSocket.handle_info({:heartbeat_tick, state.connection_nonce}, stale_state)

    assert is_reference(state.pending_heartbeat_token)
    assert is_reference(state.heartbeat_timeout_ref)

    cancel_timers(state)
  end

  test "pong clears the pending heartbeat timeout" do
    {:ok, state} = TerminalUi.PaneSocket.handle_connect(nil, %{pane_id: "pane-1"})

    stale_state = %{state | last_activity_ms: System.monotonic_time(:millisecond) - 10_000}

    {:reply, {:ping, "pane-heartbeat"}, state} =
      TerminalUi.PaneSocket.handle_info({:heartbeat_tick, state.connection_nonce}, stale_state)

    {:ok, state} = TerminalUi.PaneSocket.handle_pong({:pong, "pane-heartbeat"}, state)

    assert state.pending_heartbeat_token == nil
    assert state.heartbeat_timeout_ref == nil

    cancel_timers(state)
  end

  test "heartbeat timeout closes the socket so WebSockex reconnects" do
    {:ok, state} = TerminalUi.PaneSocket.handle_connect(nil, %{pane_id: "pane-1"})

    stale_state = %{state | last_activity_ms: System.monotonic_time(:millisecond) - 10_000}

    {:reply, {:ping, "pane-heartbeat"}, state} =
      TerminalUi.PaneSocket.handle_info({:heartbeat_tick, state.connection_nonce}, stale_state)

    {:close, {4000, "heartbeat_timeout"}, state} =
      TerminalUi.PaneSocket.handle_info(
        {:heartbeat_timeout, state.connection_nonce, state.pending_heartbeat_token},
        state
      )

    assert state.pending_heartbeat_token == nil

    cancel_timers(state)
  end

  defp cancel_timers(state) do
    Enum.each([state.heartbeat_tick_ref, state.heartbeat_timeout_ref], fn
      nil -> :ok
      timer_ref -> Process.cancel_timer(timer_ref)
    end)
  end
end
