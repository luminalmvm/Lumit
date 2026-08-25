// The toolbar's state machine (K-216): which tool is armed, what a group button
// stands for, and what pressing a tool's key twice does.
//
// Pure state, so this needs no engine and no widgets — the parts of a toolbar
// that are easy to get subtly wrong (a group forgetting the variant you chose,
// a shortcut cycling when it should not) are all here rather than in the paint.

import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/state/tools.dart';

void main() {
  group('Arming a tool', () {
    test('the session opens on the selection tool', () {
      expect(ToolsState().tool, ToolMode.select);
    });

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
    test('a group button stands for its first member until one is picked', () {
      final tools = ToolsState();
      expect(tools.memberOf(ToolGroup.shape), ToolMode.shapeRectangle);

      tools.select(ToolMode.shapeStar);
      expect(tools.memberOf(ToolGroup.shape), ToolMode.shapeStar);
    });

    test('the memory survives arming another group and coming back', () {
      // A built member, because an unbuilt one cannot be armed at all (K-228).
      final tools = ToolsState()..select(ToolMode.shapeStar);
      tools.select(ToolMode.hand);

      tools.selectGroup(ToolGroup.shape);
      expect(tools.tool, ToolMode.shapeStar,
          reason: 'pressing the button gives back the tool you last had');
    });
  });

  group('A tool chord arms, then cycles', () {
    test('the first press arms the remembered member', () {
      final tools = ToolsState()..select(ToolMode.shapeEllipse);
      tools.select(ToolMode.select);

      tools.cycleGroup(ToolGroup.shape);
      expect(tools.tool, ToolMode.shapeEllipse);
    });

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

    test('a group of one stays put however often its key is pressed', () {
      final tools = ToolsState();
      tools.cycleGroup(ToolGroup.hand);
      tools.cycleGroup(ToolGroup.hand);
      expect(tools.tool, ToolMode.hand);
    });
  });

  group('Keymap actions', () {
    test('every group has exactly one action, and every action a group', () {
      for (final group in ToolGroup.values) {
        expect(toolActions.values.where((g) => g == group).length, 1,
            reason: '$group needs one and only one chord to arm it');
      }
      // The ids are the engine's (docs/07 §15); a typo here would silently
      // leave a tool unreachable from the keyboard.
      for (final action in toolActions.keys) {
        expect(action, startsWith('tool.'));
      }
    });

    test('a tool action is handled and anything else is left alone', () {
      final tools = ToolsState();
      expect(tools.handleAction('tool.razor'), isTrue);
      expect(tools.tool, ToolMode.razor);

      expect(tools.handleAction('edit.undo'), isFalse);
      expect(tools.tool, ToolMode.razor, reason: 'and nothing moved');
    });
  });

  group('The tool set itself', () {
    test('every group has at least one tool, in declaration order', () {
      for (final group in ToolGroup.values) {
        final members = ToolMode.membersOf(group);
        expect(members, isNotEmpty, reason: '$group would be an empty button');
        expect(members.first.group, group);
      }
    });

    test('the tools that claim to be built are the ones that are', () {
      // A guard on honesty rather than on behaviour: `ready` is what the
      // tooltip promises, so it may only be true where something reads the
      // armed tool and does the work. Selection selects and drags (K-217),
      // Hand pans, Zoom magnifies (K-218), Rotation turns (K-219), Anchor
      // point pans behind and the Razor cuts (K-220), the five shape tools
      // draw masks and the Pen builds one (K-222, K-223), horizontal type makes
      // and edits text layers (K-225), the three painting tools paint, erase and
      // clone (K-227), and the three camera tools move the active camera
      // (K-229); everything else is on the strip and disabled (K-228).
      expect(ToolMode.values.where((t) => t.ready).toSet(), {
        ToolMode.select,
        ToolMode.hand,
        ToolMode.zoom,
        ToolMode.rotate,
        ToolMode.anchor,
        ToolMode.razor,
        ToolMode.shapeRectangle,
        ToolMode.shapeRoundedRectangle,
        ToolMode.shapeEllipse,
        ToolMode.shapePolygon,
        ToolMode.shapeStar,
        ToolMode.pen,
        ToolMode.typeHorizontal,
        ToolMode.brush,
        ToolMode.cloneStamp,
        ToolMode.eraser,
        ToolMode.cameraOrbit,
        ToolMode.cameraPan,
        ToolMode.cameraDolly,
      });
    });
  });

  /// The toolbar's tool options (K-225): the fill and size the drawing tools
  /// set things in, held here because they belong to the tool and not to any
  /// one panel.
  group('Tool options', () {
    test('the fill starts white and changes once per real change', () {
      final tools = ToolsState();
      var notices = 0;
      tools.addListener(() => notices++);

      expect(tools.fill, ToolColour.white);
      expect(tools.fillRgba.r, 1);
      expect(tools.fillRgba.a, 1, reason: 'a fill is opaque; Opacity is a '
          'transform property, not a fourth number in a swatch');

      tools.fill = const ToolColour(1, 0, 0);
      tools.fill = const ToolColour(1, 0, 0);
      expect(tools.fillRgba.g, 0);
      expect(notices, 1);
    });

    test('the text size is held within sane bounds', () {
      final tools = ToolsState();
      expect(tools.textSize, 72);
      tools.textSize = 0;
      expect(tools.textSize, 1, reason: 'text of no size is not text');
      tools.textSize = 100000;
      expect(tools.textSize, 2000);
    });

    test('the stroke is held even though nothing draws one yet', () {
      final tools = ToolsState();
      expect(tools.stroke, ToolColour.black);
      expect(tools.strokeWidth, 2);
      tools.strokeWidth = -4;
      expect(tools.strokeWidth, 0);
    });

    /// The brush's own three settings (K-227) — separate from the shape tools'
    /// stroke, because a brush is a different thing that happens to have a
    /// width, and because these are live while that pair is not.
    test('the brush has its own size, hardness and opacity', () {
      final tools = ToolsState();
      expect(tools.brushSize, 20);
      expect(tools.brushHardness, 80);
      expect(tools.brushOpacity, 100);

      var notices = 0;
      tools.addListener(() => notices++);
      tools.brushSize = 0;
      expect(tools.brushSize, 1, reason: 'a brush of no size marks nothing');
      tools.brushSize = 1e9;
      expect(tools.brushSize, 2000);
      tools.brushHardness = 200;
      expect(tools.brushHardness, 100);
      tools.brushOpacity = -5;
      expect(tools.brushOpacity, 0);
      tools.brushOpacity = 0;
      expect(notices, 4, reason: 'one notice per real change');
    });
  });

  /// A tool that is not built cannot be armed (K-228) — by click, by flyout or
  /// by chord. The refusal lives here rather than in the button because there
  /// are three ways in and only one of them is a button.
  group('What cannot be armed', () {
    test('an unbuilt tool is refused, and the armed one is left alone', () {
      final tools = ToolsState();
      var notices = 0;
      tools.addListener(() => notices++);

      tools.select(ToolMode.rotoBrush);
      expect(tools.tool, ToolMode.select, reason: 'nothing changed');
      expect(notices, 0, reason: 'and nobody was told anything had');

      tools.select(ToolMode.hand);
      expect(tools.tool, ToolMode.hand);
    });

    test('a chord on a group with nothing built does nothing', () {
      final tools = ToolsState();
      tools.cycleGroup(ToolGroup.puppet);
      expect(tools.tool, ToolMode.select);
      tools.cycleGroup(ToolGroup.roto);
      expect(tools.tool, ToolMode.select);
    });

    test('a chord cycles only the built members of a mixed group', () {
      final tools = ToolsState();
      // The Pen's four editing siblings are unbuilt, so its chord arms the Pen
      // and stays there rather than stepping onto a tool that does nothing.
      tools.cycleGroup(ToolGroup.pen);
      expect(tools.tool, ToolMode.pen);
      tools.cycleGroup(ToolGroup.pen);
      expect(tools.tool, ToolMode.pen);

      // Type's vertical member is unbuilt, so the same applies.
      tools.cycleGroup(ToolGroup.type);
      expect(tools.tool, ToolMode.typeHorizontal);
      tools.cycleGroup(ToolGroup.type);
      expect(tools.tool, ToolMode.typeHorizontal);
    });

    test('a group whose first member is unbuilt opens on one that works', () {
      final tools = ToolsState();
      expect(ToolMode.builtMembersOf(ToolGroup.roto), isEmpty);
      expect(tools.memberOf(ToolGroup.type), ToolMode.typeHorizontal);
      // A group with nothing built still names a member for its button to draw.
      expect(tools.memberOf(ToolGroup.puppet), ToolMode.puppetPosition);
    });
  });

}
