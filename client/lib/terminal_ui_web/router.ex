defmodule TerminalUiWeb.Router do
  use TerminalUiWeb, :router

  pipeline :browser do
    plug :accepts, ["html"]
    plug :fetch_session
    plug :fetch_live_flash
    plug :put_root_layout, html: {TerminalUiWeb.Layouts, :root}
    plug :protect_from_forgery
    plug :put_secure_browser_headers
  end

  pipeline :require_auth do
    plug TerminalUiWeb.Plugs.Auth
  end

  # Public: login / logout
  scope "/", TerminalUiWeb do
    pipe_through :browser

    get  "/login",  SessionController, :new
    post "/login",  SessionController, :create
    delete "/logout", SessionController, :delete
  end

  # Protected: everything else
  scope "/", TerminalUiWeb do
    pipe_through [:browser, :require_auth]

    live "/", TerminalLive, :index
  end
end
