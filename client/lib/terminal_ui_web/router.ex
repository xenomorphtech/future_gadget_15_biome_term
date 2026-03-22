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

  pipeline :api do
    plug :accepts, ["json"]
  end

  scope "/", TerminalUiWeb do
    pipe_through :browser

    live "/", TerminalLive, :index
  end

  # Other scopes may use custom stacks.
  # scope "/api", TerminalUiWeb do
  #   pipe_through :api
  # end
end
