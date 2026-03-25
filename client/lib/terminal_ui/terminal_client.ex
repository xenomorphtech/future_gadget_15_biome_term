defmodule TerminalUi.TerminalClient do
  @default_base "http://localhost:3021"
  @base_url_env :terminal_server_url
  @legacy_base_url_env :terminal_server_http_url
  @api_key_env :terminal_server_api_key

  def list_panes do
    Req.get!(base_url() <> "/panes", req_options()).body
  end

  def create_pane(cols \\ 220, rows \\ 50, name \\ nil) do
    body = %{cols: cols, rows: rows}
    body = if name, do: Map.put(body, :name, name), else: body
    Req.post!(base_url() <> "/panes", Keyword.merge(req_options(), json: body)).body
  end

  def kill_pane(id) do
    Req.delete!(base_url() <> "/panes/#{id}", req_options())
  end

  def get_screen(id) do
    case Req.get(base_url() <> "/panes/#{id}/screen", req_options()) do
      {:ok, %Req.Response{status: status, body: body}} when status in 200..299 ->
        {:ok, body}

      {:ok, %Req.Response{status: 404}} ->
        {:error, :not_found}

      {:ok, %Req.Response{status: status}} ->
        {:error, {:http_error, status}}

      {:error, reason} ->
        {:error, reason}
    end
  end

  def get_pane(id) do
    case Req.get(base_url() <> "/panes", req_options()) do
      {:ok, %Req.Response{status: status, body: panes}} when status in 200..299 ->
        case Enum.find(panes, &(&1["id"] == id)) do
          nil -> {:error, :not_found}
          pane -> {:ok, pane}
        end

      {:ok, %Req.Response{status: 404}} ->
        {:error, :not_found}

      {:ok, %Req.Response{status: status}} ->
        {:error, {:http_error, status}}

      {:error, reason} ->
        {:error, reason}
    end
  end

  def send_input(id, data) do
    Req.post!(
      base_url() <> "/panes/#{id}/input",
      Keyword.merge(req_options(), json: %{data: Base.encode64(data)})
    )
  end

  def ws_url(path) do
    ws_base_url() <> path
  end

  def ws_conn(path) do
    case WebSockex.Conn.new(ws_url(path), extra_headers: auth_headers()) do
      %WebSockex.Conn{} = conn -> conn
      {:error, reason} -> raise "invalid terminal websocket URL: #{inspect(reason)}"
    end
  end

  defp base_url do
    configured_string(@base_url_env) ||
      configured_string(@legacy_base_url_env) ||
      configured_string("BIOME_URL") ||
      @default_base
  end

  defp ws_base_url do
    base_url()
    |> String.replace_prefix("https://", "wss://")
    |> String.replace_prefix("http://", "ws://")
  end

  defp req_options do
    [headers: auth_headers()]
  end

  defp auth_headers do
    case api_key() do
      nil -> []
      key -> [{"authorization", "Bearer #{key}"}]
    end
  end

  defp api_key do
    configured_string(@api_key_env) || configured_string("BIOME_API_KEY")
  end

  defp configured_string(key) when is_atom(key) do
    :terminal_ui
    |> Application.get_env(key)
    |> normalize_optional_string()
  end

  defp configured_string(key) when is_binary(key) do
    key
    |> System.get_env()
    |> normalize_optional_string()
  end

  defp normalize_optional_string(nil), do: nil

  defp normalize_optional_string(value) do
    value
    |> to_string()
    |> String.trim()
    |> case do
      "" -> nil
      normalized -> normalized
    end
  end
end
