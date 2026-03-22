defmodule TerminalUiWeb.TerminalLive do
  use TerminalUiWeb, :live_view

  alias TerminalUi.{PaneSupervisor, TerminalClient}

  @impl true
  def mount(_params, _session, socket) do
    socket = assign(socket, panes: [], selected_pane_id: nil, screen: nil, new_pane_name: "")
    if connected?(socket), do: send(self(), :load_panes)
    {:ok, socket}
  end

  @impl true
  def handle_info(:load_panes, socket) do
    panes = TerminalClient.list_panes()
    Enum.each(panes, &PaneSupervisor.ensure_started(&1["id"]))
    socket = assign(socket, :panes, panes)

    socket =
      case panes do
        [%{"id" => id} | _] -> select_pane(socket, id)
        [] -> socket
      end

    {:noreply, socket}
  end

  @impl true
  def handle_info({:screen_update, id, screen}, socket) do
    if socket.assigns.selected_pane_id == id do
      {:noreply, assign(socket, :screen, screen)}
    else
      {:noreply, socket}
    end
  end

  @impl true
  def handle_event("select_pane", %{"id" => id}, socket) do
    {:noreply, select_pane(socket, id)}
  end

  @impl true
  def handle_event("set_new_pane_name", %{"value" => value}, socket) do
    {:noreply, assign(socket, :new_pane_name, value)}
  end

  @impl true
  def handle_event("clear_new_pane_name", _, socket) do
    {:noreply, assign(socket, :new_pane_name, "")}
  end

  @impl true
  def handle_event("new_pane", _, socket) do
    name = case String.trim(socket.assigns.new_pane_name) do
      "" -> nil
      n  -> n
    end
    pane = TerminalClient.create_pane(220, 50, name)
    new_id = pane["id"]
    PaneSupervisor.ensure_started(new_id)
    panes = socket.assigns.panes ++ [pane]
    socket = socket |> assign(:panes, panes) |> assign(:new_pane_name, "") |> select_pane(new_id)
    {:noreply, socket}
  end

  @impl true
  def handle_event("kill_pane", %{"id" => id}, socket) do
    TerminalClient.kill_pane(id)
    PaneSupervisor.stop(id)
    panes = Enum.reject(socket.assigns.panes, &(&1["id"] == id))
    socket = assign(socket, :panes, panes)

    socket =
      if socket.assigns.selected_pane_id == id do
        case panes do
          [%{"id" => next_id} | _] -> select_pane(socket, next_id)
          [] -> assign(socket, selected_pane_id: nil, screen: nil)
        end
      else
        socket
      end

    {:noreply, socket}
  end

  @impl true
  def handle_event("send_input", %{"key" => key}, socket) do
    if id = socket.assigns.selected_pane_id do
      TerminalClient.send_input(id, key)
    end

    {:noreply, socket}
  end

  defp select_pane(socket, new_id) do
    if old = socket.assigns.selected_pane_id do
      Phoenix.PubSub.unsubscribe(TerminalUi.PubSub, "pane:#{old}")
    end

    Phoenix.PubSub.subscribe(TerminalUi.PubSub, "pane:#{new_id}")

    screen =
      case TerminalClient.get_screen(new_id) do
        {:ok, screen} -> screen
        _ -> nil
      end

    assign(socket, selected_pane_id: new_id, screen: screen)
  end
end
