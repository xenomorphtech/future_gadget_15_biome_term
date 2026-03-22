defmodule TerminalUiWeb.PageControllerTest do
  use TerminalUiWeb.ConnCase

  test "GET /", %{conn: conn} do
    conn = get(conn, ~p"/")
    assert redirected_to(conn, 302) == ~p"/login"
  end
end
