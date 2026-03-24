defmodule TerminalUi.PaneLifecycleSocketTest do
  use ExUnit.Case, async: true

  test "rebroadcasts lifecycle frames onto Phoenix PubSub" do
    Phoenix.PubSub.subscribe(TerminalUi.PubSub, "panes")

    {:ok, %{}} =
      TerminalUi.PaneLifecycleSocket.handle_frame(
        {:text,
         ~s({"type":"snapshot","panes":[{"id":"pane-1","name":"one","cols":80,"rows":24,"terminated":false}]})},
        %{}
      )

    assert_receive {:panes_snapshot, [%{"id" => "pane-1"}]}

    {:ok, %{}} =
      TerminalUi.PaneLifecycleSocket.handle_frame(
        {:text,
         ~s({"type":"created","pane":{"id":"pane-2","name":"two","cols":80,"rows":24,"terminated":false}})},
        %{}
      )

    assert_receive {:pane_created, %{"id" => "pane-2", "name" => "two"}}

    {:ok, %{}} =
      TerminalUi.PaneLifecycleSocket.handle_frame(
        {:text, ~s({"type":"deleted","id":"pane-2"})},
        %{}
      )

    assert_receive {:pane_deleted, "pane-2"}
  end
end
