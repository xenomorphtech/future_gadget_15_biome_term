defmodule TerminalUi.TerminalClient do
  @base "http://localhost:3000"

  def list_panes do
    Req.get!(@base <> "/panes").body
  end

  def create_pane(cols \\ 220, rows \\ 50, name \\ nil) do
    body = %{cols: cols, rows: rows}
    body = if name, do: Map.put(body, :name, name), else: body
    Req.post!(@base <> "/panes", json: body).body
  end

  def kill_pane(id) do
    Req.delete!(@base <> "/panes/#{id}")
  end

  def get_screen(id) do
    {:ok, Req.get!(@base <> "/panes/#{id}/screen").body}
  end

  def send_input(id, data) do
    Req.post!(@base <> "/panes/#{id}/input", json: %{data: Base.encode64(data)})
  end
end
