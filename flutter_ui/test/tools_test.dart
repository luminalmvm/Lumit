// The toolbar's state machine: which tool is armed, what a group button
// stands for, and what pressing a tool's key twice does.
//
// Pure state, so this needs no engine and no widgets — the parts of a toolbar
// that are easy to get subtly wrong (a group forgetting the variant you chose,
// a shortcut cycling when it should not) are all here rather than in the paint.

import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/state/tools.dart';

void main() {
  group('Arming a tool', () {
    test('selecting notifies once, and re-selecting the armed tool does not',
        () {
      final tools = ToolsState();
      var notices = 0;
      tools.addListener(() => notices++);

      tools.select(ToolMode.pen);
      expect(tools.tool, ToolMode.pen);
      expect(notices, 1);

      tools.select(ToolMode.pen);
      expect(notices, 1, reason: 'nothing changed, so nothing redraws');
    });
  });

  group('Groups remember the variant you chose', () {
    test('the memory survives arming another group and coming back', () {
      // A built member, because an unbuilt one cannot be armed at all.
      final tools = ToolsState()..select(ToolMode.shapeStar);
      tools.select(ToolMode.hand);

      tools.selectGroup(ToolGroup.shape);
      expect(tools.tool, ToolMode.shapeStar,
          reason: 'pressing the button gives back the tool you last had');
    });
  });

  group('A tool chord arms, then cycles', () {
    test('pressing again steps through the group and wraps', () {
      final tools = ToolsState();
      final shapes = ToolMode.membersOf(ToolGroup.shape);
      expect(shapes.length, 5, reason: 'AE\'s five shape tools');

      tools.cycleGroup(ToolGroup.shape);
      for (var i = 1; i <= shapes.length; i++) {
        tools.cycleGroup(ToolGroup.shape);
        expect(tools.tool, shapes[i % shapes.length]);
      }
      expect(tools.tool, shapes.first, reason: 'a full lap comes home');
    });
  });

  group('Keymap actions', () {
    test('a tool action is handled and anything else is left alone', () {
      final tools = ToolsState();
      expect(tools.handleAction('tool.razor'), isTrue);
      expect(tools.tool, ToolMode.razor);

      expect(tools.handleAction('edit.undo'), isFalse);
      expect(tools.tool, ToolMode.razor, reason: 'and nothing moved');
    });
  });

  /// A tool that is not built cannot be armed — by click, by flyout or
  /// by chord. The refusal lives here rather than in the button because there
  /// are three ways in and only one of them is a button.
  group('What cannot be armed', () {
    test('an unbuilt tool is refused, and the armed one is left alone', () {
      final tools = ToolsState();
      var notices = 0;
      tools.addListener(() => notices++);

      tools.select(ToolMode.typeVertical);
      expect(tools.tool, ToolMode.select, reason: 'nothing changed');
      expect(notices, 0, reason: 'and nobody was told anything had');

      tools.select(ToolMode.hand);
      expect(tools.tool, ToolMode.hand);
    });
  });
}
