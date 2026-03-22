defmodule TerminalUiWeb.SessionController do
  use TerminalUiWeb, :controller

  def new(conn, _params) do
    render(conn, :new)
  end

  def create(conn, %{"password" => password}) do
    expected = Application.get_env(:terminal_ui, :auth_password, "changeme")

    if Plug.Crypto.secure_compare(password, expected) do
      conn
      |> put_session(:authenticated, true)
      |> redirect(to: "/")
    else
      conn
      |> put_flash(:error, "Invalid password")
      |> render(:new)
    end
  end

  def delete(conn, _params) do
    conn
    |> clear_session()
    |> redirect(to: "/login")
  end
end
