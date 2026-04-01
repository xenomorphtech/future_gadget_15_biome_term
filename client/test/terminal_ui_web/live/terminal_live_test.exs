defmodule TerminalUiWeb.TerminalLiveTest.MockTerminalState do
  use Agent

  def start_link(_opts) do
    Agent.start_link(fn -> %{panes: [], screens: %{}} end, name: __MODULE__)
  end

  def reset!(panes, screens) do
    Agent.update(__MODULE__, fn _state ->
      %{panes: panes, screens: screens}
    end)
  end

  def panes do
    Agent.get(__MODULE__, & &1.panes)
  end

  def screen(id) do
    Agent.get(__MODULE__, &get_in(&1, [:screens, id]))
  end

  def put_screen!(id, screen) do
    Agent.update(__MODULE__, fn state ->
      put_in(state, [:screens, id], screen)
    end)
  end
end

defmodule TerminalUiWeb.TerminalLiveTest.MockTerminalServer do
  use Plug.Router

  plug :match
  plug :dispatch

  get "/panes" do
    conn
    |> Plug.Conn.put_resp_content_type("application/json")
    |> Plug.Conn.send_resp(
      200,
      Jason.encode!(TerminalUiWeb.TerminalLiveTest.MockTerminalState.panes())
    )
  end

  get "/panes/:id/screen" do
    case TerminalUiWeb.TerminalLiveTest.MockTerminalState.screen(id) do
      nil ->
        Plug.Conn.send_resp(conn, 404, "")

      screen ->
        conn
        |> Plug.Conn.put_resp_content_type("application/json")
        |> Plug.Conn.send_resp(200, Jason.encode!(screen))
    end
  end

  get "/panes/:id/stream" do
    conn
    |> WebSockAdapter.upgrade(TerminalUiWeb.TerminalLiveTest.MockPaneStreamSocket, %{id: id},
      timeout: 60_000
    )
    |> Plug.Conn.halt()
  end

  match _ do
    Plug.Conn.send_resp(conn, 404, "")
  end
end

defmodule TerminalUiWeb.TerminalLiveTest.MockPaneStreamSocket do
  def init(state), do: {:ok, state}

  def handle_in({_payload, _opts}, state) do
    {:ok, state}
  end
end

defmodule TerminalUiWeb.TerminalLiveTest do
  use TerminalUiWeb.ConnCase, async: false

  import Phoenix.LiveViewTest

  setup do
    start_supervised!(TerminalUiWeb.TerminalLiveTest.MockTerminalState)

    server =
      start_supervised!(
        {Bandit, plug: TerminalUiWeb.TerminalLiveTest.MockTerminalServer, port: 0}
      )

    {:ok, {_address, port}} = ThousandIsland.listener_info(server)

    previous_url = Application.get_env(:terminal_ui, :terminal_server_url)
    Application.put_env(:terminal_ui, :terminal_server_url, "http://127.0.0.1:#{port}")

    on_exit(fn ->
      if previous_url do
        Application.put_env(:terminal_ui, :terminal_server_url, previous_url)
      else
        Application.delete_env(:terminal_ui, :terminal_server_url)
      end
    end)

    :ok
  end

  test "switching back to a pane fetches and renders its latest screen state", %{conn: conn} do
    pane_a = %{"id" => "pane-a", "name" => "Pane A", "terminated" => false, "idle_seconds" => 0}
    pane_b = %{"id" => "pane-b", "name" => "Pane B", "terminated" => false, "idle_seconds" => 0}

    on_exit(fn ->
      TerminalUi.PaneSupervisor.stop("pane-a")
      TerminalUi.PaneSupervisor.stop("pane-b")
    end)

    TerminalUiWeb.TerminalLiveTest.MockTerminalState.reset!(
      [pane_a, pane_b],
      %{
        "pane-a" => %{"rows" => ["pane-a-initial"]},
        "pane-b" => %{"rows" => ["pane-b-old"]}
      }
    )

    conn = Phoenix.ConnTest.init_test_session(conn, authenticated: true)
    {:ok, view, _html} = live(conn, "/")

    send(view.pid, :load_panes)
    assert render(view) =~ "pane-a-initial"

    view
    |> element("button[phx-click='select_pane'][phx-value-id='pane-b']")
    |> render_click()

    assert render(view) =~ "pane-b-old"

    view
    |> element("button[phx-click='select_pane'][phx-value-id='pane-a']")
    |> render_click()

    assert render(view) =~ "pane-a-initial"

    TerminalUiWeb.TerminalLiveTest.MockTerminalState.put_screen!(
      "pane-b",
      %{"rows" => ["pane-b-new"]}
    )

    view
    |> element("button[phx-click='select_pane'][phx-value-id='pane-b']")
    |> render_click()

    html = render(view)
    assert html =~ "pane-b-new"
    refute html =~ "pane-b-old"
  end
end
