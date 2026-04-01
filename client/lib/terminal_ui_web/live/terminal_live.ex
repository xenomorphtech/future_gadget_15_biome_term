defmodule TerminalUiWeb.TerminalLive do
  use TerminalUiWeb, :live_view

  require Logger

  alias TerminalUi.{PaneSupervisor, TerminalClient}

  @state_refresh_interval_ms 250
  @snippet_submit_delay_ms 500
  @idle_badge_threshold_seconds 5

  @impl true
  def mount(_params, _session, socket) do
    socket =
      assign(socket,
        panes: [],
        groups: [],
        selected_group: nil,
        pane_buffers: %{},
        selected_pane_id: nil,
        screen: nil,
        new_pane_name: "",
        snippet: "",
        snippet_panel_position: "bottom"
      )

    if connected?(socket) do
      send(self(), :load_panes)
      :timer.send_interval(@state_refresh_interval_ms, :poll_terminal_state)
    end

    {:ok, socket}
  end

  @impl true
  def handle_info(:load_panes, socket) do
    {:noreply, refresh_terminal_state(socket)}
  end

  @impl true
  def handle_info(:poll_terminal_state, socket) do
    {:noreply, refresh_polled_terminal_state(socket)}
  end

  @impl true
  def handle_info({:screen_update, id, screen}, socket) do
    {:noreply, update_pane_buffer(socket, id, screen)}
  end

  @impl true
  def handle_info({:send_snippet_enter, id}, socket) do
    if pane_exists?(socket, id) do
      send_input(id, "\r")
    end

    {:noreply, socket}
  end

  @impl true
  def handle_event("select_group", %{"group" => "all"}, socket) do
    {:noreply, socket |> assign(:selected_group, nil) |> refresh_panes()}
  end

  @impl true
  def handle_event("select_group", %{"group" => group}, socket) do
    {:noreply, socket |> assign(:selected_group, group) |> refresh_panes()}
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
  def handle_event("set_snippet_panel_position", %{"position" => position}, socket) do
    {:noreply,
     assign(socket, :snippet_panel_position, normalize_snippet_panel_position(position))}
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
      |> refresh_terminal_state()
      |> select_pane(pane["id"])
      |> assign(:new_pane_name, "")

    {:noreply, socket}
  end

  @impl true
  def handle_event("kill_pane", %{"id" => id}, socket) do
    TerminalClient.kill_pane(id)
    {:noreply, refresh_terminal_state(socket)}
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

  defp select_pane(socket, nil) do
    socket
    |> configure_selected_pane_stream(nil)
    |> assign(selected_pane_id: nil, screen: nil)
  end

  defp select_pane(socket, new_id) do
    socket
    |> configure_selected_pane_stream(new_id)
    |> assign(selected_pane_id: new_id)
    |> refresh_pane_screen(new_id)
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

  defp normalize_snippet_panel_position("top"), do: "top"
  defp normalize_snippet_panel_position(_position), do: "bottom"

  defp sync_panes(socket, panes) do
    current_ids =
      socket.assigns.panes
      |> Enum.map(& &1["id"])
      |> MapSet.new()

    next_ids =
      panes
      |> Enum.map(& &1["id"])
      |> MapSet.new()

    removed_ids = MapSet.difference(current_ids, next_ids) |> MapSet.to_list()

    Enum.each(removed_ids, &PaneSupervisor.stop/1)

    pane_buffers =
      Enum.reduce(panes, socket.assigns.pane_buffers, fn pane, acc ->
        pane_id = pane["id"]
        current_buffer = Map.get(acc, pane_id)
        Map.put(acc, pane_id, sync_pane_buffer(pane, current_buffer))
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
    load_pane_buffer(id, nil)
  end

  defp load_pane_buffer(id, fallback) do
    case TerminalClient.get_screen(id) do
      {:ok, screen} ->
        Map.merge(
          fallback_buffer(fallback),
          %{
            screen: screen
          }
        )

      _ ->
        fallback_buffer(fallback)
    end
  end

  defp ensure_pane_buffer_loaded(socket, pane_id) do
    case Map.fetch(socket.assigns.pane_buffers, pane_id) do
      {:ok, %{screen: nil} = pane_buffer} ->
        loaded_buffer = load_pane_buffer(pane_id, pane_buffer)
        {put_pane_buffer(socket, pane_id, loaded_buffer), loaded_buffer}

      {:ok, pane_buffer} ->
        {socket, pane_buffer}

      :error ->
        pane_buffer = load_pane_buffer(pane_id)
        {put_pane_buffer(socket, pane_id, pane_buffer), pane_buffer}
    end
  end

  defp refresh_terminal_state(socket) do
    socket =
      socket
      |> refresh_panes()
      |> refresh_selected_screen()

    refresh_pane_buffer_idle(socket)
  end

  defp refresh_polled_terminal_state(socket) do
    socket
    |> refresh_panes()
    |> ensure_selected_pane_stream()
    |> refresh_selected_screen()
  end

  defp refresh_panes(socket) do
    all_panes = TerminalClient.list_panes()
    groups = all_panes |> Enum.map(& &1["group"]) |> Enum.reject(&is_nil/1) |> Enum.uniq() |> Enum.sort()

    filtered_panes =
      case socket.assigns.selected_group do
        nil -> all_panes
        group -> Enum.filter(all_panes, &(&1["group"] == group))
      end

    socket
    |> assign(:groups, groups)
    |> sync_panes(filtered_panes)
    |> refresh_pane_buffer_idle()
  end

  defp refresh_selected_screen(socket) do
    selected_pane_id = socket.assigns.selected_pane_id

    cond do
      is_nil(selected_pane_id) ->
        socket

      not pane_exists?(socket, selected_pane_id) ->
        socket

      true ->
        refresh_pane_screen(socket, selected_pane_id)
    end
  end

  defp refresh_pane_screen(socket, pane_id) do
    case TerminalClient.get_screen(pane_id) do
      {:ok, screen} ->
        update_pane_buffer(socket, pane_id, screen)

      _ ->
        pane_buffer = Map.get(socket.assigns.pane_buffers, pane_id, fallback_buffer(nil))
        socket = put_pane_buffer(socket, pane_id, pane_buffer)

        if socket.assigns.selected_pane_id == pane_id do
          assign(socket, :screen, pane_buffer.screen)
        else
          socket
        end
    end
  end

  defp put_pane_buffer(socket, pane_id, pane_buffer) do
    assign(socket, :pane_buffers, Map.put(socket.assigns.pane_buffers, pane_id, pane_buffer))
  end

  defp sync_pane_buffer(pane, current_buffer) do
    idle_seconds = Map.get(pane, "idle_seconds", 0)
    now_ms = System.monotonic_time(:millisecond)

    %{
      screen: current_buffer && current_buffer.screen,
      idle_since_ms: now_ms - idle_seconds * 1_000,
      idle_seconds: idle_seconds
    }
  end

  defp fallback_buffer(nil) do
    %{
      screen: nil,
      idle_since_ms: nil,
      idle_seconds: 0
    }
  end

  defp fallback_buffer(buffer), do: buffer

  defp configure_selected_pane_stream(socket, new_id) do
    current_id = socket.assigns.selected_pane_id

    cond do
      not connected?(socket) ->
        socket

      current_id == new_id ->
        socket

      true ->
        if current_id do
          Phoenix.PubSub.unsubscribe(TerminalUi.PubSub, "pane:#{current_id}")
        end

        if new_id do
          Phoenix.PubSub.subscribe(TerminalUi.PubSub, "pane:#{new_id}")
          ensure_pane_stream_started(new_id)
        end

        socket
    end
  end

  defp ensure_selected_pane_stream(socket) do
    pane_id = socket.assigns.selected_pane_id

    cond do
      not connected?(socket) ->
        socket

      is_nil(pane_id) ->
        socket

      not pane_exists?(socket, pane_id) ->
        socket

      true ->
        ensure_pane_stream_started(pane_id)
        socket
    end
  end

  defp ensure_pane_stream_started(pane_id) do
    case PaneSupervisor.ensure_started(pane_id) do
      :ok ->
        :ok

      {:error, reason} ->
        Logger.warning("failed to start pane stream for #{pane_id}: #{inspect(reason)}")
        :ok
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

  defp pane_group(pane), do: pane["group"]

  defp ensure_valid_selection(socket) do
    case socket.assigns.panes do
      [] ->
        select_pane(socket, nil)

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

  attr :selected_pane_id, :string, default: nil
  attr :snippet, :string, required: true
  attr :position, :string, required: true

  def snippet_panel(assigns) do
    ~H"""
    <form
      id="snippet-form"
      phx-change="set_snippet"
      phx-submit="inject_snippet"
      phx-hook="SnippetInput"
      data-history-scope={@selected_pane_id || ""}
      class={[
        "bg-gray-950 p-3",
        if(@position == "top", do: "border-b border-gray-800", else: "border-t border-gray-800")
      ]}
    >
      <div class="flex items-end gap-3">
        <div class="flex-1">
          <div class="mb-2 flex items-center justify-between gap-3">
            <label
              for="snippet-input"
              class="block text-xs font-semibold uppercase tracking-wide text-gray-400"
            >
              Inject Snippet
            </label>
            <div class="flex flex-wrap items-center justify-end gap-2">
              <span class="text-[11px] text-gray-500">Alt+↑ / Alt+↓ history</span>
              <div class="flex items-center rounded border border-gray-700 p-0.5">
                <button
                  type="button"
                  phx-click="set_snippet_panel_position"
                  phx-value-position="top"
                  class={
                    if @position == "top" do
                      "rounded bg-blue-600 px-2 py-1 text-[11px] font-semibold text-white"
                    else
                      "rounded px-2 py-1 text-[11px] font-semibold text-gray-400 hover:text-gray-200"
                    end
                  }
                >
                  Top
                </button>
                <button
                  type="button"
                  phx-click="set_snippet_panel_position"
                  phx-value-position="bottom"
                  class={
                    if @position == "bottom" do
                      "rounded bg-blue-600 px-2 py-1 text-[11px] font-semibold text-white"
                    else
                      "rounded px-2 py-1 text-[11px] font-semibold text-gray-400 hover:text-gray-200"
                    end
                  }
                >
                  Bottom
                </button>
              </div>
              <button
                type="button"
                data-history-nav="prev"
                disabled={is_nil(@selected_pane_id)}
                class="rounded border border-gray-700 px-2 py-1 text-[11px] font-semibold text-gray-300 disabled:cursor-not-allowed disabled:border-gray-800 disabled:text-gray-600"
              >
                Prev
              </button>
              <button
                type="button"
                data-history-nav="next"
                disabled={is_nil(@selected_pane_id)}
                class="rounded border border-gray-700 px-2 py-1 text-[11px] font-semibold text-gray-300 disabled:cursor-not-allowed disabled:border-gray-800 disabled:text-gray-600"
              >
                Next
              </button>
            </div>
          </div>
          <textarea
            id="snippet-input"
            name="snippet"
            rows="5"
            class="w-full resize-y rounded border border-gray-700 bg-gray-900 px-3 py-2 text-sm text-gray-100 outline-none focus:border-blue-500 focus:ring-1 focus:ring-blue-500"
            placeholder="Paste multiline input here. It will be sent to the selected pane through the input API."
          ><%= @snippet %></textarea>
        </div>
        <button
          type="submit"
          disabled={is_nil(@selected_pane_id) or @snippet == ""}
          class={
            if is_nil(@selected_pane_id) or @snippet == "" do
              "shrink-0 rounded bg-gray-700 px-4 py-2 text-sm font-semibold text-gray-400 cursor-not-allowed"
            else
              "shrink-0 rounded bg-blue-600 px-4 py-2 text-sm font-semibold text-white hover:bg-blue-500"
            end
          }
        >
          Send
        </button>
      </div>
    </form>
    """
  end
end
