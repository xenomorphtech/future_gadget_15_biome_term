defmodule TerminalUiWeb.TerminalLive do
  use TerminalUiWeb, :live_view

  alias TerminalUi.{PaneSupervisor, TerminalClient}

  @snippet_submit_delay_ms 500
  @idle_badge_threshold_seconds 5

  @impl true
  def mount(_params, _session, socket) do
    socket =
      assign(socket,
        panes: [],
        pane_buffers: %{},
        selected_pane_id: nil,
        screen: nil,
        new_pane_name: "",
        snippet: ""
      )

    if connected?(socket) do
      Phoenix.PubSub.subscribe(TerminalUi.PubSub, "panes")
      send(self(), :load_panes)
      :timer.send_interval(1_000, :refresh_screen_idle)
    end

    {:ok, socket}
  end

  @impl true
  def handle_info(:load_panes, socket) do
    {:noreply, sync_panes(socket, TerminalClient.list_panes())}
  end

  @impl true
  def handle_info({:panes_snapshot, panes}, socket) do
    {:noreply, sync_panes(socket, panes)}
  end

  @impl true
  def handle_info({:pane_created, pane}, socket) do
    {:noreply, upsert_pane(socket, pane)}
  end

  @impl true
  def handle_info({:pane_deleted, id}, socket) do
    {:noreply, remove_pane(socket, id)}
  end

  @impl true
  def handle_info({:pane_terminated, id}, socket) do
    panes =
      Enum.map(socket.assigns.panes, fn pane ->
        if pane["id"] == id, do: Map.put(pane, "terminated", true), else: pane
      end)

    {:noreply, assign(socket, :panes, panes)}
  end

  @impl true
  def handle_info({:screen_update, id, screen}, socket) do
    {:noreply, update_pane_buffer(socket, id, screen)}
  end

  @impl true
  def handle_info(:refresh_screen_idle, socket) do
    {:noreply, refresh_pane_buffer_idle(socket)}
  end

  @impl true
  def handle_info({:send_snippet_enter, id}, socket) do
    if pane_exists?(socket, id) do
      send_input(id, "\r")
    end

    {:noreply, socket}
  end

  @impl true
  def handle_event("select_pane", %{"id" => id}, socket) do
    {:noreply, select_pane(socket, id)}
  end

  @impl true
  def handle_event("set_new_pane_name", %{"pane_name" => value}, socket) do
    {:noreply, assign(socket, :new_pane_name, value)}
  end

  @impl true
  def handle_event("set_snippet", %{"snippet" => snippet}, socket) do
    {:noreply, assign(socket, :snippet, snippet)}
  end

  @impl true
  def handle_event("new_pane", %{"pane_name" => value}, socket) do
    name =
      case String.trim(value) do
        "" -> nil
        n -> n
      end

    pane = TerminalClient.create_pane(220, 50, name)

    socket =
      socket
      |> upsert_pane(pane, select: true)
      |> assign(:new_pane_name, "")

    {:noreply, socket}
  end

  @impl true
  def handle_event("kill_pane", %{"id" => id}, socket) do
    TerminalClient.kill_pane(id)
    {:noreply, remove_pane(socket, id)}
  end

  @impl true
  def handle_event("send_input", %{"key" => key}, socket) do
    send_input(socket.assigns.selected_pane_id, key)

    {:noreply, socket}
  end

  @impl true
  def handle_event("inject_snippet", %{"snippet" => snippet}, socket) do
    send_snippet_input(socket.assigns.selected_pane_id, snippet)

    socket =
      if socket.assigns.selected_pane_id && snippet != "" do
        assign(socket, :snippet, "")
      else
        assign(socket, :snippet, snippet)
      end

    {:noreply, socket}
  end

  defp select_pane(socket, new_id) do
    {socket, pane_buffer} = ensure_pane_buffer_loaded(socket, new_id)

    assign(socket,
      selected_pane_id: new_id,
      screen: pane_buffer.screen
    )
  end

  defp send_input(nil, _data), do: :ok
  defp send_input(_id, ""), do: :ok

  defp send_input(id, data) do
    TerminalClient.send_input(id, data)
  end

  defp send_snippet_input(nil, _snippet), do: :ok
  defp send_snippet_input(_id, ""), do: :ok

  defp send_snippet_input(id, snippet) do
    {body, final_enter} = normalize_snippet_input(snippet)

    send_input(id, body)

    if final_enter != "" do
      Process.send_after(self(), {:send_snippet_enter, id}, @snippet_submit_delay_ms)
    end
  end

  defp normalize_snippet_input(snippet) do
    normalized =
      String.replace(snippet, ~r/\r\n|\n|\r/u, "\r")

    {strip_one_trailing_enter(normalized), "\r"}
  end

  defp strip_one_trailing_enter(snippet) do
    if String.ends_with?(snippet, "\r"),
      do: binary_part(snippet, 0, byte_size(snippet) - 1),
      else: snippet
  end

  defp pane_exists?(socket, id) do
    Enum.any?(socket.assigns.panes, &(&1["id"] == id))
  end

  defp sync_panes(socket, panes) do
    current_ids =
      socket.assigns.panes
      |> Enum.map(& &1["id"])
      |> MapSet.new()

    next_ids =
      panes
      |> Enum.map(& &1["id"])
      |> MapSet.new()

    added_panes = Enum.reject(panes, &MapSet.member?(current_ids, &1["id"]))
    removed_ids = MapSet.difference(current_ids, next_ids) |> MapSet.to_list()

    Enum.each(added_panes, fn pane ->
      PaneSupervisor.ensure_started(pane["id"])
      subscribe_to_pane_updates(pane["id"])
    end)

    Enum.each(removed_ids, fn pane_id ->
      PaneSupervisor.stop(pane_id)
      unsubscribe_from_pane_updates(pane_id)
    end)

    pane_buffers =
      Enum.reduce(panes, socket.assigns.pane_buffers, fn pane, acc ->
        pane_id = pane["id"]

        if Map.has_key?(acc, pane_id) do
          acc
        else
          Map.put(acc, pane_id, load_pane_buffer(pane_id))
        end
      end)
      |> Map.take(Enum.map(panes, & &1["id"]))

    socket =
      assign(socket,
        panes: panes,
        pane_buffers: pane_buffers
      )

    ensure_valid_selection(socket)
  end

  defp load_pane_buffer(id) do
    case TerminalClient.get_screen(id) do
      {:ok, screen} ->
        now_ms = System.monotonic_time(:millisecond)

        %{
          screen: screen,
          idle_since_ms: now_ms,
          idle_seconds: 0
        }

      _ ->
        %{
          screen: nil,
          idle_since_ms: nil,
          idle_seconds: 0
        }
    end
  end

  defp ensure_pane_buffer_loaded(socket, pane_id) do
    case Map.fetch(socket.assigns.pane_buffers, pane_id) do
      {:ok, pane_buffer} ->
        {socket, pane_buffer}

      :error ->
        pane_buffer = load_pane_buffer(pane_id)
        {put_pane_buffer(socket, pane_id, pane_buffer), pane_buffer}
    end
  end

  defp put_pane_buffer(socket, pane_id, pane_buffer) do
    assign(socket, :pane_buffers, Map.put(socket.assigns.pane_buffers, pane_id, pane_buffer))
  end

  defp upsert_pane(socket, pane, opts \\ []) do
    pane_id = pane["id"]

    socket =
      if pane_exists?(socket, pane_id) do
        panes =
          Enum.map(socket.assigns.panes, fn current_pane ->
            if current_pane["id"] == pane_id,
              do: Map.merge(current_pane, pane),
              else: current_pane
          end)

        assign(socket, :panes, panes)
      else
        PaneSupervisor.ensure_started(pane_id)
        subscribe_to_pane_updates(pane_id)

        socket
        |> assign(:panes, socket.assigns.panes ++ [pane])
        |> put_pane_buffer(pane_id, load_pane_buffer(pane_id))
      end

    cond do
      opts[:select] ->
        select_pane(socket, pane_id)

      is_nil(socket.assigns.selected_pane_id) ->
        select_pane(socket, pane_id)

      true ->
        socket
    end
  end

  defp remove_pane(socket, id) do
    if pane_exists?(socket, id) do
      PaneSupervisor.stop(id)
      unsubscribe_from_pane_updates(id)
      panes = Enum.reject(socket.assigns.panes, &(&1["id"] == id))

      socket =
        assign(socket,
          panes: panes,
          pane_buffers: Map.delete(socket.assigns.pane_buffers, id)
        )

      ensure_valid_selection(socket)
    else
      socket
    end
  end

  defp update_pane_buffer(socket, pane_id, screen) do
    current_buffer =
      Map.get(socket.assigns.pane_buffers, pane_id, %{
        screen: nil,
        idle_since_ms: nil,
        idle_seconds: 0
      })

    now_ms = System.monotonic_time(:millisecond)

    idle_since_ms =
      if buffer_content_changed?(current_buffer.screen, screen) or
           is_nil(current_buffer.idle_since_ms) do
        now_ms
      else
        current_buffer.idle_since_ms
      end

    pane_buffer = %{
      screen: screen,
      idle_since_ms: idle_since_ms,
      idle_seconds: idle_seconds_since(idle_since_ms, now_ms)
    }

    socket = put_pane_buffer(socket, pane_id, pane_buffer)

    if socket.assigns.selected_pane_id == pane_id do
      assign(socket, :screen, screen)
    else
      socket
    end
  end

  defp refresh_pane_buffer_idle(socket) do
    now_ms = System.monotonic_time(:millisecond)

    pane_buffers =
      Enum.into(socket.assigns.pane_buffers, %{}, fn {pane_id, pane_buffer} ->
        updated_buffer =
          case pane_buffer.idle_since_ms do
            nil ->
              pane_buffer

            idle_since_ms ->
              %{pane_buffer | idle_seconds: idle_seconds_since(idle_since_ms, now_ms)}
          end

        {pane_id, updated_buffer}
      end)

    assign(socket, :pane_buffers, pane_buffers)
  end

  defp buffer_content_changed?(nil, _screen), do: true

  defp buffer_content_changed?(current_screen, next_screen) do
    Map.get(current_screen, "rows") != Map.get(next_screen, "rows")
  end

  defp idle_seconds_since(idle_since_ms, now_ms) do
    max(div(now_ms - idle_since_ms, 1_000), 0)
  end

  defp pane_idle_seconds(pane_buffers, pane_id) do
    pane_buffers
    |> Map.get(pane_id, %{idle_seconds: 0})
    |> Map.get(:idle_seconds, 0)
  end

  defp show_idle_badge?(pane, pane_buffers) do
    not pane["terminated"] and
      pane_idle_seconds(pane_buffers, pane["id"]) > @idle_badge_threshold_seconds
  end

  defp pane_display_name(pane) do
    pane["name"] || String.slice(pane["id"], 0, 8) <> "…"
  end

  defp ensure_valid_selection(socket) do
    case socket.assigns.panes do
      [] ->
        assign(socket, selected_pane_id: nil, screen: nil)

      [%{"id" => first_id} | _] ->
        cond do
          is_nil(socket.assigns.selected_pane_id) ->
            select_pane(socket, first_id)

          pane_exists?(socket, socket.assigns.selected_pane_id) ->
            {socket, pane_buffer} =
              ensure_pane_buffer_loaded(socket, socket.assigns.selected_pane_id)

            assign(socket, :screen, pane_buffer.screen)

          true ->
            select_pane(socket, first_id)
        end
    end
  end

  defp subscribe_to_pane_updates(pane_id) do
    Phoenix.PubSub.subscribe(TerminalUi.PubSub, "pane:#{pane_id}")
  end

  defp unsubscribe_from_pane_updates(pane_id) do
    Phoenix.PubSub.unsubscribe(TerminalUi.PubSub, "pane:#{pane_id}")
  end
end
