defmodule TerminalUi.PaneSupervisor do
  use DynamicSupervisor

  def start_link(opts) do
    DynamicSupervisor.start_link(__MODULE__, opts, name: __MODULE__)
  end

  @impl true
  def init(_opts) do
    DynamicSupervisor.init(strategy: :one_for_one)
  end

  def ensure_started(pane_id) do
    case Registry.lookup(TerminalUi.PaneRegistry, pane_id) do
      [{_pid, _}] ->
        :ok

      [] ->
        spec = {TerminalUi.PaneSocket, pane_id}

        case DynamicSupervisor.start_child(__MODULE__, spec) do
          {:ok, _pid} -> :ok
          {:error, {:already_started, _pid}} -> :ok
          {:error, reason} -> {:error, reason}
        end
    end
  end

  def stop(pane_id) do
    case Registry.lookup(TerminalUi.PaneRegistry, pane_id) do
      [{pid, _}] -> DynamicSupervisor.terminate_child(__MODULE__, pid)
      [] -> :ok
    end
  end
end
