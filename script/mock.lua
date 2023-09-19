function fast_accel()
  hold("L", 1)
  hold("B", 1)
  hold("R", "A", 1)
end

function jump(height)
  hold("A", height)
end

-- player state
-- 0 = ground
PLAYER_STATE = 0x1D
GROUNDED = 0

FRAME = 60

function jumps(n, height)
  for i = 0, n do
    while read(PLAYER_STATE) ~= GROUNDED do
      wait(1)
    end
    hold("A", height)
  end
end

function ground()
  while read(PLAYER_STATE) ~= GROUNDED do
    wait(1)
  end
end

fast_accel()
press("R", "B")

-- jump over first goomba
wait(60)
jump(60)

-- jump over pipes
wait(10)
jump(13)

jumps(2, 30)

-- go into pipe
ground()
wait(5)
release("R")
hold("D", 1)
