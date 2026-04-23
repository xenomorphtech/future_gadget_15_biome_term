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

  def record_resize!(id, cols, rows) do
    Agent.update(__MODULE__, fn state ->
      state
      |> Map.update(:resizes, [{id, cols, rows}], &(&1 ++ [{id, cols, rows}]))
      |> update_in([:panes], fn panes ->
        Enum.map(panes, fn pane ->
          if pane["id"] == id,
            do: Map.merge(pane, %{"cols" => cols, "rows" => rows}),
            else: pane
        end)
      end)
    end)
  end

  def resizes do
    Agent.get(__MODULE__, &Map.get(&1, :resizes, []))
  end

  def set_resize_status!(status) do
    Agent.update(__MODULE__, &Map.put(&1, :resize_status, status))
  end

  def resize_status do
    Agent.get(__MODULE__, &Map.get(&1, :resize_status, 204))
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

  post "/panes/:id/resize" do
    {:ok, body, conn} = Plug.Conn.read_body(conn)
    %{"cols" => cols, "rows" => rows} = Jason.decode!(body)
    status = TerminalUiWeb.TerminalLiveTest.MockTerminalState.resize_status()

    if status in 200..299 do
      TerminalUiWeb.TerminalLiveTest.MockTerminalState.record_resize!(id, cols, rows)
      Plug.Conn.send_resp(conn, status, "")
    else
      Plug.Conn.send_resp(conn, status, "")
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

  test "terminal size modal resizes only the current pane when scope is current", %{conn: conn} do
    pane_a = %{
      "id" => "pane-a",
      "name" => "Pane A",
      "terminated" => false,
      "idle_seconds" => 0,
      "cols" => 80,
      "rows" => 24
    }

    pane_b = %{
      "id" => "pane-b",
      "name" => "Pane B",
      "terminated" => false,
      "idle_seconds" => 0,
      "cols" => 80,
      "rows" => 24
    }

    on_exit(fn ->
      TerminalUi.PaneSupervisor.stop("pane-a")
      TerminalUi.PaneSupervisor.stop("pane-b")
    end)

    TerminalUiWeb.TerminalLiveTest.MockTerminalState.reset!(
      [pane_a, pane_b],
      %{"pane-a" => %{"rows" => ["a"]}, "pane-b" => %{"rows" => ["b"]}}
    )

    conn = Phoenix.ConnTest.init_test_session(conn, authenticated: true)
    {:ok, view, _html} = live(conn, "/")
    send(view.pid, :load_panes)
    render(view)

    view |> element("button[phx-click='open_size_settings']") |> render_click()
    assert render(view) =~ "Terminal size"

    view
    |> form("form[phx-submit='apply_size_settings']", %{
      "cols" => "120",
      "rows" => "40",
      "scope" => "current"
    })
    |> render_submit()

    assert TerminalUiWeb.TerminalLiveTest.MockTerminalState.resizes() == [{"pane-a", 120, 40}]
  end

  test "terminal size modal resizes every pane when scope is all", %{conn: conn} do
    pane_a = %{
      "id" => "pane-a",
      "name" => "Pane A",
      "terminated" => false,
      "idle_seconds" => 0,
      "cols" => 80,
      "rows" => 24
    }

    pane_b = %{
      "id" => "pane-b",
      "name" => "Pane B",
      "terminated" => false,
      "idle_seconds" => 0,
      "cols" => 80,
      "rows" => 24
    }

    on_exit(fn ->
      TerminalUi.PaneSupervisor.stop("pane-a")
      TerminalUi.PaneSupervisor.stop("pane-b")
    end)

    TerminalUiWeb.TerminalLiveTest.MockTerminalState.reset!(
      [pane_a, pane_b],
      %{"pane-a" => %{"rows" => ["a"]}, "pane-b" => %{"rows" => ["b"]}}
    )

    conn = Phoenix.ConnTest.init_test_session(conn, authenticated: true)
    {:ok, view, _html} = live(conn, "/")
    send(view.pid, :load_panes)
    render(view)

    view |> element("button[phx-click='open_size_settings']") |> render_click()

    view
    |> form("form[phx-submit='apply_size_settings']", %{
      "cols" => "100",
      "rows" => "30",
      "scope" => "all"
    })
    |> render_submit()

    resizes = TerminalUiWeb.TerminalLiveTest.MockTerminalState.resizes()
    assert Enum.sort(resizes) == [{"pane-a", 100, 30}, {"pane-b", 100, 30}]
  end

  test "terminal size modal rejects out-of-range values", %{conn: conn} do
    pane_a = %{
      "id" => "pane-a",
      "name" => "Pane A",
      "terminated" => false,
      "idle_seconds" => 0,
      "cols" => 80,
      "rows" => 24
    }

    on_exit(fn -> TerminalUi.PaneSupervisor.stop("pane-a") end)

    TerminalUiWeb.TerminalLiveTest.MockTerminalState.reset!(
      [pane_a],
      %{"pane-a" => %{"rows" => ["a"]}}
    )

    conn = Phoenix.ConnTest.init_test_session(conn, authenticated: true)
    {:ok, view, _html} = live(conn, "/")
    send(view.pid, :load_panes)
    render(view)

    view |> element("button[phx-click='open_size_settings']") |> render_click()

    html =
      view
      |> form("form[phx-submit='apply_size_settings']", %{
        "cols" => "1",
        "rows" => "30",
        "scope" => "current"
      })
      |> render_submit()

    assert html =~ "cols must be between"
    assert TerminalUiWeb.TerminalLiveTest.MockTerminalState.resizes() == []
  end

  test "terminal size modal surfaces backend errors and keeps modal open", %{conn: conn} do
    pane_a = %{
      "id" => "pane-a",
      "name" => "Pane A",
      "terminated" => false,
      "idle_seconds" => 0,
      "cols" => 80,
      "rows" => 24
    }

    on_exit(fn -> TerminalUi.PaneSupervisor.stop("pane-a") end)

    TerminalUiWeb.TerminalLiveTest.MockTerminalState.reset!(
      [pane_a],
      %{"pane-a" => %{"rows" => ["a"]}}
    )

    TerminalUiWeb.TerminalLiveTest.MockTerminalState.set_resize_status!(500)

    conn = Phoenix.ConnTest.init_test_session(conn, authenticated: true)
    {:ok, view, _html} = live(conn, "/")
    send(view.pid, :load_panes)
    render(view)

    view |> element("button[phx-click='open_size_settings']") |> render_click()

    html =
      view
      |> form("form[phx-submit='apply_size_settings']", %{
        "cols" => "120",
        "rows" => "40",
        "scope" => "current"
      })
      |> render_submit()

    assert html =~ "HTTP 500"
    assert html =~ "Terminal size"
    assert TerminalUiWeb.TerminalLiveTest.MockTerminalState.resizes() == []
  end
end
