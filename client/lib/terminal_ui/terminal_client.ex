defmodule TerminalUi.TerminalClient do
  @default_base "http://localhost:3000"

  def list_panes do
    Req.get!(base_url() <> "/panes").body
  end

  def create_pane(cols \\ 220, rows \\ 50, name \\ nil) do
    body = %{cols: cols, rows: rows}
    body = if name, do: Map.put(body, :name, name), else: body
    Req.post!(base_url() <> "/panes", json: body).body
  end

  def kill_pane(id) do
    Req.delete!(base_url() <> "/panes/#{id}")
  end

  def get_screen(id) do
    {:ok, Req.get!(base_url() <> "/panes/#{id}/screen").body}
  end

  def get_pane(id) do
    panes = Req.get!(base_url() <> "/panes").body

    case Enum.find(panes, &(&1["id"] == id)) do
      nil -> {:error, :not_found}
      pane -> {:ok, pane}
    end
  end

  def send_input(id, data) do
    Req.post!(base_url() <> "/panes/#{id}/input", json: %{data: Base.encode64(data)})
  end

  def ws_url(path) do
    ws_base_url() <> path
  end

  defp base_url do
    Application.get_env(:terminal_ui, :terminal_server_http_url, @default_base)
  end

  defp ws_base_url do
    base_url()
    |> String.replace_prefix("https://", "wss://")
    |> String.replace_prefix("http://", "ws://")
  end
end
